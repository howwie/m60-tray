//! Edifier M60 control: protocol, device state and the connection state machine.
//!
//! The Bluetooth backend is plugged in through the [`controller::Radio`] and
//! [`controller::Link`] traits, so everything here is testable without hardware.

pub mod controller;
pub mod proto;
pub mod state;

pub use controller::{Advert, Command, Controller, Handle, Link, Radio, Settings};
pub use state::{Resume, State};
