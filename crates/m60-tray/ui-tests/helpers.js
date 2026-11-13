const fs = require('fs');
const path = require('path');

/** Save a screenshot when SCREENSHOTS_DIR is set (handy for reviewing the look); otherwise do nothing. */
async function shot(page, name) {
  const dir = process.env.SCREENSHOTS_DIR;
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: path.join(dir, name) });
}

module.exports = { shot };
