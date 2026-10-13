import { _android, expect } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const serial = process.env.ANDROID_SERIAL || 'emulator-5554';
if (!/^emulator-\d+$/.test(serial)) throw new Error('This smoke test only targets an isolated emulator');
const adb = process.env.ANDROID_ADB || path.join(process.env.ANDROID_HOME || '', 'platform-tools/adb.exe');
const out = path.resolve('../../artifacts/android');
fs.mkdirSync(out, { recursive: true });
function android(...args) { return execFileSync(adb, ['-s', serial, ...args], { encoding: 'utf8', timeout: 30000 }); }
function appPid() { try { return android('shell', 'pidof', 'com.changjin.sidetask').trim(); } catch { return ''; } }
async function connect() {
  await expect.poll(appPid, { timeout: 30000 }).toMatch(/^\d+$/);
  const pid = appPid();
  if (!/^\d+$/.test(pid)) throw new Error('SideTask is not running');
  const devices = await _android.devices();
  const device = devices.find(device => device.serial() === serial);
  if (!device) throw new Error('Emulator was not returned by the Android driver');
  const webview = await device.webView({ pkg: 'com.changjin.sidetask' });
  const page = await webview.page();
  const browser = { close: () => device.close() };
  await expect(page.getByRole('navigation', { name: '主导航' })).toBeVisible({ timeout: 30000 });
  return { browser, page };
}
const call = (page, command, args = {}) => page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
let { browser, page } = await connect();
if (process.argv.includes('--inspect')) {
  console.log(JSON.stringify({ url: page.url(), body: (await page.locator('body').innerText()).slice(0, 1800), snapshot: await call(page, 'get_snapshot'), sync: await call(page, 'sync_status') }));
  await browser.close();
  process.exit(0);
}
if (process.argv.includes('--reset')) {
  const snapshot = await call(page, 'get_snapshot');
  const status = await call(page, 'sync_status');
  if (status.enabled || status.email || snapshot.tasks.some(task => !/^Android emulator task(?: [23])?$/.test(task.title))) throw new Error('Refusing to reset anything except the isolated synthetic smoke database');
  if (!android('emu','avd','name').startsWith('SideTask_API35')) throw new Error('Unexpected AVD; reset refused');
  await browser.close();
  android('shell','pm','clear','com.changjin.sidetask');
  android('shell','am','start','-n','com.changjin.sidetask/.MainActivity');
  ({ browser, page } = await connect());
}
const report = { serial, tests: [], passed: false, coverage: 'Actual Android WebView, Tauri commands, SQLite and Keystore. Synthetic tasks only. No user cloud account.' };
async function step(name, action) { await action(); report.tests.push(name); console.log(`PASS ${name}`); }
try {
  await step('native database starts without demo tasks', async () => {
    expect((await call(page, 'get_snapshot')).tasks).toHaveLength(0);
  });
  await step('create, edit, complete and remove from today without changing deadline', async () => {
    await page.getByRole('button', { name: '新建任务', exact: true }).click();
    await page.getByRole('textbox', { name: '任务名称', exact: true }).fill('Android emulator task');
    await page.getByRole('textbox', { name: '任务备注' }).fill('模拟器中文备注');
    await page.getByLabel('截止日期', { exact: true }).fill('2027-02-10');
    await page.getByRole('button', { name: '保存任务', exact: true }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    let snapshot = await call(page, 'get_snapshot');
    expect(snapshot.tasks).toHaveLength(1);
    const task = snapshot.tasks[0];
    expect(task.notes).toBe('模拟器中文备注');
    expect(task.dueDate).toBe('2027-02-10');
    await page.getByRole('button', { name: '完成：Android emulator task', exact: true }).click();
    await expect(page.getByRole('button', { name: '撤销完成：Android emulator task', exact: true })).toBeVisible();
    expect((await call(page, 'get_snapshot')).tasks[0].completed).toBe(true);
    await page.getByRole('button', { name: '撤销完成：Android emulator task', exact: true }).click();
    await expect(page.getByRole('button', { name: '完成：Android emulator task', exact: true })).toBeVisible();
    await page.getByRole('button', { name: '将Android emulator task移出今日' }).click();
    await expect(page.getByRole('button', { name: '编辑任务：Android emulator task', exact: true })).toHaveCount(0);
    snapshot = await call(page, 'get_snapshot');
    expect(snapshot.tasks[0].completed).toBe(false);
    expect(snapshot.tasks[0].dueDate).toBe('2027-02-10');
    expect(snapshot.plans.some(plan => plan.taskId === task.id)).toBe(false);
  });
  await step('draft retained by Android back and stored across process restart', async () => {
    await page.getByRole('button', { name: '新建任务', exact: true }).click();
    await page.getByRole('textbox', { name: '任务名称', exact: true }).fill('Native restart draft');
    android('shell', 'input', 'keyevent', '4');
    // If the soft keyboard consumes the first Back, close the editor explicitly.
    if (await page.getByRole('dialog').isVisible()) await page.getByRole('button', { name: '收起编辑并保留草稿' }).click();
    await expect.poll(async () => (await call(page, 'mobile_load_draft'))?.title).toBe('Native restart draft');
    expect(await call(page, 'mobile_credential_probe', { action: 'write' })).toBe(true);
    const encrypted = android('exec-out','run-as','com.changjin.sidetask','cat','shared_prefs/sidetask_sessions.xml');
    expect(encrypted).not.toContain('synthetic-android-probe-access');
    expect(encrypted).not.toContain('synthetic-android-probe-refresh');
    expect(encrypted).toContain('<string');
    await browser.close();
    android('shell','am','force-stop','com.changjin.sidetask');
    android('shell','am','start','-n','com.changjin.sidetask/.MainActivity');
    await expect.poll(appPid, { timeout: 30000 }).toMatch(/^\d+$/);
    ({ browser, page } = await connect());
    expect((await call(page, 'get_snapshot')).tasks[0].title).toBe('Android emulator task');
    expect(await call(page, 'mobile_credential_probe', { action: 'read' })).toBe(true);
    expect(await call(page, 'mobile_credential_probe', { action: 'delete' })).toBe(true);
    await page.getByRole('button', { name: /继续未保存的新任务/ }).click();
    await expect(page.getByRole('textbox', { name: '任务名称', exact: true })).toHaveValue('Native restart draft');
    await page.getByRole('button', { name: '放弃草稿', exact: true }).click();
  });
  await step('native credential bridge cannot be read from JavaScript', async () => {
    let rejected = false;
    try { await call(page, 'plugin:sidetask-mobile|readSession', { key: '0'.repeat(64) }); } catch { rejected = true; }
    expect(rejected).toBe(true);
  });
  if (process.env.SIDETASK_TEST_PROJECT_URL && process.env.SIDETASK_TEST_PUBLISHABLE_KEY) await step('Android system certificate TLS reaches Supabase auth settings', async () => {
    expect(await call(page, 'mobile_network_probe', { projectUrl: process.env.SIDETASK_TEST_PROJECT_URL, publishableKey: process.env.SIDETASK_TEST_PUBLISHABLE_KEY })).toBe(200);
  });
  await step('task survives trash restore and lifecycle return', async () => {
    await page.getByRole('button', { name: '全部', exact: true }).click();
    await page.getByRole('button', { name: '编辑任务：Android emulator task', exact: true }).click();
    await page.getByRole('button', { name: '移入回收站', exact: true }).click();
    await page.getByRole('button', { name: '设置', exact: true }).click();
    await page.getByRole('button', { name: /回收站/ }).click();
    await page.getByRole('button', { name: '恢复任务：Android emulator task' }).click();
    android('shell','input','keyevent','3');
    android('shell','am','start','-n','com.changjin.sidetask/.MainActivity');
    await expect.poll(async () => (await call(page, 'get_snapshot')).tasks[0].deletedAt ?? null).toBeNull();
  });
  await step('phone layout does not overflow', async () => {
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.getByRole('button', { name: '截止', exact: true }).click();
    await expect(page.getByRole('button', { name: '编辑任务：Android emulator task', exact: true })).toBeVisible();
  });
  await step('native Android touch reorders without changing deadlines', async () => {
    for (const suffix of ['2','3']) {
      await page.getByRole('button', { name: '新建任务', exact: true }).click();
      await page.getByRole('textbox', { name: '任务名称', exact: true }).fill(`Android emulator task ${suffix}`);
      await page.getByRole('dialog').getByLabel('截止日期', { exact: true }).fill('2027-02-10');
      await page.getByRole('button', { name: '保存任务', exact: true }).click();
      await expect(page.getByRole('dialog')).toHaveCount(0);
    }
    await page.getByRole('button', { name: '全部', exact: true }).click();
    const before = await call(page, 'get_snapshot');
    const rows = page.getByRole('list', { name: '全部任务', exact: true }).getByRole('listitem');
    const first = await rows.first().getAttribute('data-task-list-id');
    const handle = await rows.first().getByRole('button', { name: /拖动排序/ }).boundingBox();
    const last = await rows.last().boundingBox();
    android('shell','uiautomator','dump','/sdcard/sidetask-smoke-ui.xml');
    const xml = android('shell','cat','/sdcard/sidetask-smoke-ui.xml');
    const bounds = xml.match(/class="android.webkit.WebView"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/);
    if (!bounds || !handle || !last) throw new Error('Cannot measure native touch coordinates');
    const scale = (Number(bounds[3]) - Number(bounds[1])) / await page.evaluate(() => innerWidth);
    const x = Math.round(Number(bounds[1]) + (handle.x + handle.width / 2) * scale);
    android('shell','input','swipe',String(x),String(Math.round(Number(bounds[2]) + (handle.y + handle.height / 2) * scale)),String(x),String(Math.round(Number(bounds[2]) + (last.y + last.height - 5) * scale)),'650');
    await expect.poll(async () => (await call(page, 'get_snapshot')).taskOrder.at(-1)).toBe(first);
    const after = await call(page, 'get_snapshot');
    expect(after.tasks.map(task => task.dueDate)).toEqual(before.tasks.map(task => task.dueDate));
    expect(after.deadlineOrder).toEqual(before.deadlineOrder);
  });
  report.passed = true;
} finally {
  fs.writeFileSync(path.join(out, 'emulator-smoke.json'), JSON.stringify(report, null, 2));
  try { fs.writeFileSync(path.join(out, 'emulator-tasks.png'), execFileSync(adb, ['-s',serial,'exec-out','screencap','-p'])); } catch {}
  await browser.close().catch(() => {});
}
