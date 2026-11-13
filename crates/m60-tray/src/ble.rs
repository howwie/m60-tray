//! Bluetooth backend: implements `m60-core`'s `Radio` / `Link` with btleplug
//! (WinRT on Windows, CoreBluetooth on macOS, BlueZ on Linux).

use std::pin::Pin;
use std::time::{Duration, Instant};

use btleplug::api::{Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures_util::stream::{Stream, StreamExt};
use m60_core::proto;
use m60_core::{Advert, Link, Radio};

type NotificationStream = Pin<Box<dyn Stream<Item = btleplug::api::ValueNotification> + Send>>;
type EventStream = Pin<Box<dyn Stream<Item = CentralEvent> + Send>>;

pub struct BleRadio {
    adapter: Option<Adapter>,
}

impl BleRadio {
    pub fn new() -> Self {
        BleRadio { adapter: None }
    }

    /// The first Bluetooth adapter, looked up lazily so the app starts fine without Bluetooth.
    async fn adapter(&mut self) -> Result<Adapter, String> {
        if let Some(a) = &self.adapter {
            return Ok(a.clone());
        }
        let manager = Manager::new().await.map_err(|e| e.to_string())?;
        let adapter = manager
            .adapters()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| "no Bluetooth adapter found (is Bluetooth turned on?)".to_string())?;
        self.adapter = Some(adapter.clone());
        Ok(adapter)
    }
}

async fn collect(adapter: &Adapter) -> Result<Vec<Advert>, String> {
    let mut out = Vec::new();
    for p in adapter.peripherals().await.map_err(|e| e.to_string())? {
        let Ok(Some(props)) = p.properties().await else { continue };
        out.push(Advert {
            id: p.id().to_string(),
            services: props.services.iter().map(|u| u.as_u128()).collect(),
            manufacturer_ids: props.manufacturer_data.keys().copied().collect(),
            rssi: props.rssi,
        });
    }
    Ok(out)
}

impl Radio for BleRadio {
    type Link = BleLink;

    async fn scan(&mut self, timeout: Duration, prefer: Option<&str>) -> Result<Vec<Advert>, String> {
        let adapter = self.adapter().await?;
        if let Err(e) = adapter.start_scan(ScanFilter::default()).await {
            self.adapter = None; // forget a stale handle (e.g. Bluetooth was switched off and on)
            return Err(e.to_string());
        }
        let deadline = Instant::now() + timeout;
        let result = loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            match collect(&adapter).await {
                Ok(adverts) => {
                    let seen_preferred = prefer.is_some_and(|p| adverts.iter().any(|a| a.id == p));
                    if seen_preferred || Instant::now() >= deadline {
                        break Ok(adverts);
                    }
                }
                Err(e) => break Err(e),
            }
        };
        let _ = adapter.stop_scan().await;
        if result.is_err() {
            self.adapter = None;
        }
        result
    }

    async fn connect(&mut self, id: &str) -> Result<BleLink, String> {
        let adapter = self.adapter().await?;
        let mut found = None;
        for p in adapter.peripherals().await.map_err(|e| e.to_string())? {
            if p.id().to_string() == id {
                found = Some(p);
                break;
            }
        }
        let peripheral = found.ok_or_else(|| "the speaker is no longer visible".to_string())?;
        match open(&adapter, &peripheral).await {
            Ok(link) => Ok(link),
            Err(e) => {
                let _ = peripheral.disconnect().await; // do not leave a half-open connection behind
                Err(e)
            }
        }
    }
}

async fn open(adapter: &Adapter, peripheral: &Peripheral) -> Result<BleLink, String> {
    let err = |what: &str| {
        let what = what.to_string();
        move |e: btleplug::Error| format!("{what}: {e}")
    };
    // A previous attempt that timed out may have left the connection half open.
    if peripheral.is_connected().await.unwrap_or(false) {
        let _ = peripheral.disconnect().await;
    }
    peripheral.connect().await.map_err(err("connect"))?;
    peripheral.discover_services().await.map_err(err("service discovery"))?;

    let chars = peripheral.characteristics();
    let find = |uuid: &str| chars.iter().find(|c| c.uuid.to_string() == uuid).cloned();
    let notify = find(proto::NOTIFY_UUID).ok_or("notify characteristic not found - is this an M60?")?;
    let write = find(proto::WRITE_UUID).ok_or("write characteristic not found - is this an M60?")?;

    // Open both streams before subscribing so nothing the speaker sends first is lost.
    let notifications = peripheral.notifications().await.map_err(err("notifications"))?;
    let events = adapter.events().await.map_err(err("events"))?;
    peripheral.subscribe(&notify).await.map_err(err("subscribe"))?;

    Ok(BleLink {
        id: peripheral.id(),
        peripheral: peripheral.clone(),
        notify,
        write,
        notifications,
        events: Some(events),
    })
}

pub struct BleLink {
    id: PeripheralId,
    peripheral: Peripheral,
    notify: Characteristic,
    write: Characteristic,
    notifications: NotificationStream,
    events: Option<EventStream>,
}

impl Link for BleLink {
    async fn write(&mut self, data: &[u8]) -> Result<(), String> {
        self.peripheral.write(&self.write, data, WriteType::WithoutResponse).await.map_err(|e| e.to_string())
    }

    async fn next_data(&mut self) -> Option<Vec<u8>> {
        let notifications = &mut self.notifications;
        let events = &mut self.events;
        loop {
            tokio::select! {
                n = notifications.next() => match n {
                    Some(n) if n.uuid == self.notify.uuid => return Some(n.value),
                    Some(_) => {}
                    None => return None,
                },
                e = async {
                    match events.as_mut() {
                        Some(s) => s.next().await,
                        None => std::future::pending().await,
                    }
                } => match e {
                    Some(CentralEvent::DeviceDisconnected(id)) if id == self.id => return None,
                    Some(_) => {}
                    None => *events = None, // event stream ended; the notification stream still works
                },
            }
        }
    }

    async fn is_connected(&mut self) -> bool {
        self.peripheral.is_connected().await.unwrap_or(false)
    }

    async fn close(&mut self) {
        let _ = self.peripheral.unsubscribe(&self.notify).await;
        let _ = self.peripheral.disconnect().await;
    }
}
