// Playwright script to record the Skrills HTML Portal demo (slower pacing)
import { chromium } from 'playwright';
import { execSync } from 'child_process';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const portalPath = resolve(__dirname, '../../skrills-portal.html');
const videoDir = resolve(__dirname, '../gifs');
const outputGif = resolve(videoDir, 'portal-demo.gif');

async function main() {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    recordVideo: { dir: videoDir, size: { width: 1280, height: 800 } },
    colorScheme: 'dark',
  });

  const page = await context.newPage();
  await page.goto('file://' + portalPath);
  await page.waitForLoadState('networkidle');

  // -- Dashboard: stats, exemplar, quick actions --
  await pause(3000);

  // Scroll to see exemplar skill and quick actions
  await smoothScroll(page, 350);
  await pause(2500);

  // Click Validate All Skills
  const validateBtn = page.locator('text=Validate All Skills');
  if (await validateBtn.isVisible()) {
    await validateBtn.click();
    await pause(2000);
  }

  // Scroll to see the report
  await smoothScroll(page, 250);
  await pause(2500);

  // Back to top
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: 'smooth' }));
  await pause(1500);

  // -- Assets Browser: Skills --
  await page.click('[data-view="assets"]');
  await pause(2500);

  // Click the exemplar skill card
  const skillCard = page.locator('.skill-card').first();
  if (await skillCard.isVisible()) {
    await skillCard.click();
    await pause(3000);
  }

  // -- Assets Browser: Commands --
  await page.selectOption('#asset-type-select', 'command');
  await pause(2000);
  const cmdCard = page.locator('.skill-card').first();
  if (await cmdCard.isVisible()) {
    await cmdCard.click();
    await pause(2500);
  }

  // -- Assets Browser: Agents --
  await page.selectOption('#asset-type-select', 'agent');
  await pause(2000);
  const agentCard = page.locator('.skill-card').first();
  if (await agentCard.isVisible()) {
    await agentCard.click();
    await pause(2500);
  }

  // -- Assets Browser: Hooks --
  await page.selectOption('#asset-type-select', 'hook');
  await pause(2000);
  const hookCard = page.locator('.skill-card').first();
  if (await hookCard.isVisible()) {
    await hookCard.click();
    await pause(2500);
  }

  // -- Creator --
  await page.click('[data-view="creator"]');
  await pause(1500);
  await typeSlowly(page, '#creator-name', 'my-demo-skill');
  await pause(500);
  await typeSlowly(page, '#creator-desc', 'A demo skill created in the portal');
  await pause(500);
  await page.fill('#creator-body', '## Instructions\n\n1. Read requirements\n2. Generate output\n3. Validate results');
  await pause(1000);
  await page.click('#btn-generate-skill');
  await pause(2500);

  // -- Validator --
  await page.click('[data-view="validate"]');
  await pause(1500);
  await page.click('#btn-load-sample');
  await pause(1500);
  await page.click('#btn-validate');
  await pause(2500);

  // Autofix
  await page.click('#btn-autofix');
  await pause(3000);

  // -- Token Analyzer --
  await page.click('[data-view="tokens"]');
  await pause(1500);
  await page.click('#btn-token-sample');
  await pause(1000);
  await page.click('#btn-analyze-tokens');
  await pause(2500);

  // -- Converter --
  await page.click('[data-view="converter"]');
  await pause(1500);
  await page.fill('#convert-input', '---\nname: demo-skill\ndescription: Example skill\n---\n\nSome instructions here.');
  await pause(1000);
  await page.click('#btn-convert-all');
  await pause(2500);

  // -- Sync --
  await page.click('[data-view="sync"]');
  await pause(2500);
  await smoothScroll(page, 300);
  await pause(2000);
  await page.evaluate(() => window.scrollTo({ top: 0, behavior: 'smooth' }));
  await pause(1000);

  // -- MCP Tools --
  await page.click('[data-view="tools"]');
  await pause(2500);

  // -- CLI Reference --
  await page.click('[data-view="cli"]');
  await pause(2500);

  // -- Back to Dashboard, toggle themes --
  await page.click('[data-view="dashboard"]');
  await pause(1500);
  await page.click('#theme-toggle'); // light
  await pause(1500);
  await page.click('#theme-toggle'); // system
  await pause(1500);
  await page.click('#theme-toggle'); // dark
  await pause(2000);

  // Close
  await page.close();
  const video = await page.video();
  const videoPath = await video.path();
  await context.close();
  await browser.close();

  console.log('Video saved to:', videoPath);

  // Convert to GIF
  try {
    execSync(`ffmpeg -y -i "${videoPath}" -vf "fps=10,scale=960:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=bayer" -loop 0 "${outputGif}"`, { stdio: 'inherit' });
    console.log('GIF saved to:', outputGif);
    const fs = await import('fs');
    fs.unlinkSync(videoPath);
  } catch (e) {
    console.error('ffmpeg conversion failed:', e.message);
    console.log('Video file available at:', videoPath);
  }
}

function pause(ms) {
  return new Promise(r => setTimeout(r, ms));
}

async function smoothScroll(page, amount) {
  await page.evaluate((px) => window.scrollBy({ top: px, behavior: 'smooth' }), amount);
  await pause(600);
}

async function typeSlowly(page, selector, text) {
  await page.click(selector);
  for (const char of text) {
    await page.keyboard.type(char, { delay: 40 });
  }
}

main().catch(console.error);
