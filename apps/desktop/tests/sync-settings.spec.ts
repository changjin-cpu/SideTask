import { expect, test, type Page } from '@playwright/test';
import { createSeed } from '../src/lib/domain';
import type { SyncStatus } from '../src/lib/sync';
import type { UIStyle } from '../src/lib/types';

// This is a native IPC simulation, never a real Supabase account or request.
// Backend protocol, isolation and durable credentials have separate Rust tests.
const disabled: SyncStatus = { phase: 'disabled', enabled: false, email: null, projectUrl: null, lastSyncedAt: null, pending: false, error: null, conflicts: [] };
const connected: SyncStatus = { ...disabled, phase: 'idle', enabled: true, email: 'synthetic@example.test', projectUrl: 'https://synthetic-project.supabase.co', lastSyncedAt: '2026-10-04T04:00:00Z' };
const password = 'Synthetic-password-no-storage-827!';
const publicKey = 'sb_publishable_synthetic_public_client_key';
const conflicts: SyncStatus = { ...connected, phase: 'conflict', pending: true, conflicts: [
  { id: 'task-a', label: '阅读报告', local: { title: '本机报告', completed: false }, remote: { title: '云端报告', completed: true } },
  { id: 'plan-order', label: '今日计划顺序', local: ['报告', '练习'], remote: ['练习', '报告'] },
] };

async function nativeSync(page: Page, initialStatus = disabled, style: UIStyle = 'paper', failSubscription = false) {
  const initialSnapshot = createSeed();
  initialSnapshot.settings.uiStyle = style;
  await page.addInitScript(({ initialSnapshot, initialStatus, failSubscription }) => {
    let snapshot = initialSnapshot;
    let status = initialStatus;
    let sequence = 0;
    const callbacks = new Map<number, (value: unknown) => void>();
    const listeners = new Map<number, { event: string; handler: number }>();
    const harness = {
      calls: [] as { command: string; args: Record<string, any> }[],
      failLogin: false, failDisconnect: false, holdRead: false, holdLogin: false, failSubscription,
      releaseRead: () => {}, releaseLogin: () => {},
      snapshot: () => structuredClone(snapshot),
      update(next: typeof status) {
        status = structuredClone(next);
        for (const [id, listener] of listeners) if (listener.event === 'sidetask:sync-changed') callbacks.get(listener.handler)?.({ id, event: listener.event, payload: {} });
      },
    };
    Object.assign(window, {
      __sync: harness,
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: (_event: string, id: number) => listeners.delete(id) },
      __TAURI_INTERNALS__: {
        transformCallback(callback: (value: unknown) => void) { const id = ++sequence; callbacks.set(id, callback); return id; },
        async invoke(command: string, args: Record<string, any> = {}) {
          harness.calls.push({ command, args });
          if (command === 'plugin:event|listen') {
            if (args.event === 'sidetask:sync-changed' && harness.failSubscription) throw new Error('合成同步事件订阅失败');
            const id = ++sequence; listeners.set(id, args as { event: string; handler: number }); return id;
          }
          if (command === 'plugin:event|unlisten') { listeners.delete(args.eventId); return; }
          if (command === 'get_snapshot') return structuredClone(snapshot);
          if (command === 'get_usage_guide_seen') return true;
          if (command === 'get_startup_recovery' || command === 'get_pending_exit') return null;
          if (command === 'get_window_status' || command === 'get_console_position_status') return { pending: false, error: null };
          if (command === 'window_action' || command === 'resolve_exit') return;
          if (command === 'mutate') {
            const modulePath = '/src/lib/domain.ts';
            const { applyPreviewAction } = await import(modulePath);
            snapshot = applyPreviewAction(snapshot, args.action, args.expectedRevision);
            return structuredClone(snapshot);
          }
          if (command === 'sync_status') {
            const result = structuredClone(status);
            if (harness.holdRead) { harness.holdRead = false; await new Promise<void>(resolve => { harness.releaseRead = resolve; }); }
            return result;
          }
          if (command === 'sync_sign_in') {
            if (harness.holdLogin) await new Promise<void>(resolve => { harness.releaseLogin = resolve; });
            if (harness.failLogin) throw new Error('合成账号登录失败，请核对邮箱和密码');
            status = { ...status, enabled: true, phase: 'idle', email: args.email, projectUrl: args.projectUrl, pending: false, lastSyncedAt: '2026-10-04T04:00:00Z', error: null, conflicts: [] };
            return structuredClone(status);
          }
          if (command === 'sync_sign_out') {
            if (harness.failDisconnect) throw new Error('合成凭据清理失败，请重试');
            status = { ...status, phase: 'disabled', enabled: false, email: null, projectUrl: null, lastSyncedAt: null, pending: false, error: null, conflicts: [] };
            return structuredClone(status);
          }
          if (command === 'sync_now' || command === 'sync_resolve') {
            status = { ...status, phase: 'idle', pending: false, error: null, conflicts: [], lastSyncedAt: '2026-10-04T05:00:00Z' };
            return structuredClone(status);
          }
          throw new Error(`Unexpected sync IPC: ${command}`);
        },
      },
    });
  }, { initialSnapshot, initialStatus, failSubscription });
  await page.goto('/?surface=console&page=settings');
  await expect(page.getByRole('heading', { name: 'Mac、Windows 与安卓同步' })).toBeVisible();
  if (!failSubscription) await expect(page.locator('.sync-load-error')).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as any).__sync.calls.filter((call: any) => call.command === 'sync_status').length)).toBeGreaterThan(0);
}
async function fillConfig(page: Page) {
  await page.getByLabel('同步项目地址', { exact: true }).fill(connected.projectUrl!);
  await page.getByLabel('同步公开连接 Key', { exact: true }).fill(publicKey);
  await page.getByLabel('同步账号邮箱', { exact: true }).fill(connected.email!);
  await page.getByLabel('同步账号密码', { exact: true }).fill(password);
}
async function calls(page: Page, command: string) {
  return page.evaluate(command => (window as any).__sync.calls.filter((call: any) => call.command === command).map((call: any) => call.args), command);
}
async function assertNoCredentialStorage(page: Page) {
  const persisted = await page.evaluate(() => JSON.stringify({ local: { ...localStorage }, session: { ...sessionStorage } }));
  expect(persisted).not.toContain(password);
  expect(persisted).not.toContain(publicKey);
  expect(persisted).not.toContain(connected.email!);
  expect(persisted).not.toContain(connected.projectUrl!);
}

test('浏览器预览明确禁用云同步，不显示凭据表单或请求云端', async ({ page }) => {
  const external: string[] = [];
  page.on('request', request => { if (/supabase\.(co|com)/.test(request.url())) external.push(request.url()); });
  await page.goto('/?surface=console&page=settings');
  await expect(page.getByText('云同步仅在桌面版使用。当前浏览器预览仍只保存在本机。')).toBeVisible();
  await expect(page.getByLabel('同步账号密码')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '登录并启用同步' })).toHaveCount(0);
  expect(external).toEqual([]);
});

test('连接校验拒绝非项目地址与特权 key；确认合并前不登录，也不阻塞保存外观设置', async ({ page }) => {
  await nativeSync(page);
  await fillConfig(page);
  const login = page.getByRole('button', { name: '登录并启用同步', exact: true });
  for (const url of ['http://synthetic-project.supabase.co', 'https://supabase.co.attacker.test', 'https://synthetic-project.supabase.co/path']) {
    await page.getByLabel('同步项目地址').fill(url);
    await login.click();
    await expect(page.locator('.sync-operation-error')).toContainText('有效的 Supabase 项目地址');
  }
  // A bad optional sync draft cannot trip the parent settings form's validity.
  await page.getByLabel('小窗宽度', { exact: true }).fill('417');
  await page.getByRole('button', { name: '保存设置', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__sync.snapshot().settings.panelWidth)).toBe(417);
  await page.getByLabel('同步项目地址').fill(connected.projectUrl!);
  const privileged = `e30.${Buffer.from(JSON.stringify({ role: 'service_role' })).toString('base64url')}.synthetic`;
  for (const key of ['sb_secret_synthetic', privileged]) {
    await page.getByLabel('同步公开连接 Key').fill(key);
    await login.click();
    await expect(page.locator('.sync-operation-error')).toContainText('不能使用 secret 或 service_role');
  }
  await page.getByLabel('同步公开连接 Key').fill(publicKey);
  await login.click();
  await expect(page.locator('.sync-operation-error')).toContainText('请先确认将本机任务与此账号合并');
  expect(await calls(page, 'sync_sign_in')).toEqual([]);
  await assertNoCredentialStorage(page);
});

test('登录失败保留非密码配置并清空密码；重试连接同账号，密码与项目配置不写浏览器存储', async ({ page }) => {
  await nativeSync(page);
  await fillConfig(page);
  await page.getByLabel('将本机任务与此账号合并', { exact: true }).check();
  await page.evaluate(() => { (window as any).__sync.failLogin = true; });
  await page.getByLabel('同步账号密码').press('Enter');
  await expect(page.locator('.sync-operation-error')).toContainText('合成账号登录失败');
  await expect(page.getByLabel('同步账号密码')).toHaveValue('');
  await expect(page.getByLabel('同步项目地址')).toHaveValue(connected.projectUrl!);
  await expect(page.getByLabel('同步账号邮箱')).toHaveValue(connected.email!);
  await expect(page.getByLabel('同步公开连接 Key')).toHaveValue(publicKey);
  expect(await calls(page, 'mutate')).toEqual([]);
  await assertNoCredentialStorage(page);
  await page.evaluate(() => { (window as any).__sync.failLogin = false; (window as any).__sync.holdLogin = true; });
  await page.getByLabel('同步账号密码').fill(password);
  await page.getByRole('button', { name: '登录并启用同步' }).click();
  await expect(page.getByLabel('同步项目地址')).toBeDisabled();
  await expect(page.getByRole('button', { name: '登录并启用同步' })).toBeDisabled();
  await page.evaluate(() => (window as any).__sync.releaseLogin());
  await expect(page.locator('.sync-summary')).toContainText(connected.email!);
  await expect(page.locator('.sync-state')).toContainText('已同步');
  await expect(page.getByLabel('同步账号密码')).toHaveCount(0);
  expect(await calls(page, 'sync_sign_in')).toEqual(Array(2).fill({ projectUrl: connected.projectUrl, publishableKey: publicKey, email: connected.email, password, mergeLocal: true }));
  await assertNoCredentialStorage(page);
});

test('连接配置草稿阻止静默离开，保存设置或回车均不会隐式登录', async ({ page }) => {
  await nativeSync(page);
  await fillConfig(page);
  await page.getByLabel('同步账号邮箱').press('Enter');
  expect(await calls(page, 'sync_sign_in')).toEqual([]);
  expect(await calls(page, 'mutate')).toEqual([]);
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: /^全部任务/ }).click();
  const dialog = page.getByRole('dialog', { name: '保留正在编辑的内容？' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: '保存并离开' }).click();
  await expect(dialog).toBeVisible();
  expect(await calls(page, 'sync_sign_in')).toEqual([]);
  await dialog.getByRole('button', { name: '继续编辑' }).click();
  await expect(page.locator('.sync-operation-error')).toContainText('密码不会随设置保存');
  await expect(page.getByLabel('同步账号密码')).toHaveValue(password);
  await page.getByRole('navigation', { name: '主导航' }).getByRole('button', { name: /^全部任务/ }).click();
  await dialog.getByRole('button', { name: '放弃修改' }).click();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await expect(page.getByLabel('同步账号密码')).toHaveValue('');
  await expect(page.getByLabel('同步项目地址')).toHaveValue('');
  await assertNoCredentialStorage(page);
});

test('同步事件更新待上传和错误；较晚返回的旧状态不能盖住新状态，可重试', async ({ page }) => {
  await nativeSync(page, connected);
  await expect(page.locator('.sync-state')).toContainText('已同步');
  await page.evaluate(status => { const h = (window as any).__sync; h.holdRead = true; h.update({ ...status, pending: true }); }, connected);
  await expect.poll(async () => (await calls(page, 'sync_status')).length).toBeGreaterThan(1);
  await page.evaluate(status => (window as any).__sync.update({ ...status, phase: 'error', pending: true, error: '合成离线错误：本机修改已保留' }), connected);
  await expect(page.locator('.sync-settings [role=alert]')).toContainText('合成离线错误');
  await page.evaluate(() => (window as any).__sync.releaseRead());
  await expect(page.locator('.sync-settings [role=alert]')).toContainText('合成离线错误');
  await expect(page.locator('.sync-state')).toContainText('待上传');
  await page.getByRole('button', { name: '重试同步', exact: true }).click();
  await expect(page.locator('.sync-state')).toContainText('已同步');
  await expect(page.locator('.sync-settings [role=alert]')).toHaveCount(0);
  expect(await calls(page, 'sync_now')).toHaveLength(1);
});

test('事件订阅失败仍提示状态可能过期，重新读取会恢复订阅并接收后续变化', async ({ page }) => {
  await nativeSync(page, connected, 'paper', true);
  await expect(page.locator('.sync-load-error')).toContainText('合成同步事件订阅失败');
  await expect(page.locator('.sync-summary')).toContainText(connected.email!);
  await page.evaluate(() => { (window as any).__sync.failSubscription = false; });
  await page.getByRole('button', { name: '重新读取同步状态', exact: true }).click();
  await expect(page.locator('.sync-load-error')).toHaveCount(0);
  await page.evaluate(status => (window as any).__sync.update({ ...status, pending: true }), connected);
  await expect(page.locator('.sync-state')).toContainText('待上传');
});

test('每项冲突必须明确选择全部版本；内容更新清除旧选择，确认只提交指定选择', async ({ page }) => {
  await nativeSync(page, conflicts);
  const resolve = page.getByRole('button', { name: '确认解决全部冲突', exact: true });
  await expect(resolve).toBeDisabled();
  await expect(page.getByRole('button', { name: '立即同步', exact: true })).toBeDisabled();
  const task = page.getByRole('group', { name: '阅读报告', exact: true });
  const plan = page.getByRole('group', { name: '今日计划顺序', exact: true });
  await expect(task).toContainText('本机报告');
  await expect(task).toContainText('云端报告');
  await task.getByRole('radio', { name: /保留本机/ }).check();
  await expect(resolve).toBeDisabled();
  await plan.getByRole('radio', { name: /保留云端/ }).check();
  await expect(resolve).toBeEnabled();
  await page.evaluate(status => (window as any).__sync.update(status), { ...conflicts, conflicts: conflicts.conflicts.map((item, i) => i ? item : { ...item, remote: { title: '新云端报告', completed: false } }) });
  await expect(task).toContainText('新云端报告');
  await expect(resolve).toBeDisabled();
  await expect(task.getByRole('radio', { name: /保留本机/ })).not.toBeChecked();
  await task.getByRole('radio', { name: /保留本机/ }).check();
  await plan.getByRole('radio', { name: /保留云端/ }).check();
  await resolve.click();
  await expect(page.getByRole('group', { name: '处理同步冲突', exact: true })).toHaveCount(0);
  expect(await calls(page, 'sync_resolve')).toEqual([{ choices: { 'task-a': 'local', 'plan-order': 'remote' } }]);
  expect(await calls(page, 'sync_now')).toEqual([]);
});

test('长备注冲突保留共同前缀后的完整差异，双方内容可用键盘滚动到尾部且阅读不代替选择', async ({ page }) => {
  const prefix = Array.from({ length: 60 }, (_, i) => `共同备注第 ${i + 1} 行：这段内容在两台电脑完全相同。`).join('\n');
  const local = `${prefix}\n本机尾部：保留现场调研结论。`;
  const remote = `${prefix}\n云端尾部：改用最新访谈结果。`;
  expect(local.slice(0, 700)).toBe(remote.slice(0, 700));
  await nativeSync(page, { ...connected, phase: 'conflict', conflicts: [{ id: 'long-notes', label: '报告备注', local, remote }] });
  const resolve = page.getByRole('button', { name: '确认解决全部冲突', exact: true });
  for (const [side, full] of [['本机', local], ['云端', remote]] as const) {
    const content = page.getByRole('region', { name: `${side}完整内容：报告备注`, exact: true });
    await expect(content).toHaveText(full);
    expect(await content.evaluate(element => element.scrollHeight > element.clientHeight)).toBe(true);
    await content.focus();
    await expect(content).toBeFocused();
    await content.press('End');
    await expect.poll(() => content.evaluate(element => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(2);
    await expect(resolve).toBeDisabled();
  }
  const group = page.getByRole('group', { name: '报告备注', exact: true });
  await expect(group.getByRole('radio', { name: '保留本机', exact: true })).not.toBeChecked();
  await expect(group.getByRole('radio', { name: '保留云端', exact: true })).not.toBeChecked();
  await group.getByRole('radio', { name: '保留云端', exact: true }).check();
  await resolve.click();
  expect(await calls(page, 'sync_resolve')).toEqual([{ choices: { 'long-notes': 'remote' } }]);
});

test('凭据失效保留账号供重新登录，兼容 anon key；断开需确认并保留本机任务', async ({ page }) => {
  await nativeSync(page, { ...connected, phase: 'signedOut', error: '凭据已失效，请重新登录' });
  await expect(page.getByLabel('同步账号邮箱')).toHaveValue(connected.email!);
  await expect(page.getByLabel('同步项目地址')).toHaveValue(connected.projectUrl!);
  await expect(page.getByText('登录已失效，请重新登录原项目与账号。需要更换账号时，请先断开同步。')).toBeVisible();
  const anon = `e30.${Buffer.from(JSON.stringify({ role: 'anon' })).toString('base64url')}.synthetic`;
  await page.getByLabel('同步公开连接 Key').fill(anon);
  await page.getByLabel('同步账号密码').fill(password);
  await page.getByLabel('将本机任务与此账号合并', { exact: true }).check();
  await page.getByRole('button', { name: '登录并启用同步' }).click();
  await expect(page.locator('.sync-summary')).toContainText(connected.email!);
  const before = await page.evaluate(() => (window as any).__sync.snapshot());
  await page.getByRole('button', { name: '断开同步', exact: true }).click();
  const confirmation = page.getByRole('group', { name: '确认断开同步', exact: true });
  await expect(confirmation).toContainText('保留本机任务，云端数据也不会删除');
  expect(await calls(page, 'sync_sign_out')).toEqual([]);
  await confirmation.getByRole('button', { name: '继续同步', exact: true }).click();
  await expect(confirmation).toHaveCount(0);
  await page.getByRole('button', { name: '断开同步', exact: true }).click();
  await page.evaluate(() => { (window as any).__sync.failDisconnect = true; });
  await confirmation.getByRole('button', { name: '确认断开', exact: true }).click();
  await expect(page.locator('.sync-operation-error')).toContainText('合成凭据清理失败');
  await expect(page.locator('.sync-summary')).toContainText(connected.email!);
  await page.evaluate(() => { (window as any).__sync.failDisconnect = false; });
  await confirmation.getByRole('button', { name: '确认断开', exact: true }).click();
  await expect(page.getByLabel('同步账号密码')).toHaveValue('');
  expect(await page.evaluate(() => (window as any).__sync.snapshot())).toEqual(before);
  await assertNoCredentialStorage(page);
});

test('停止上传后系统凭据清理失败仍提供重试断开入口，完成前保留账号与任务', async ({ page }) => {
  await nativeSync(page, { ...connected, phase: 'disabled', enabled: false, error: '系统凭据尚未清理，请重试断开' });
  const before = await page.evaluate(() => (window as any).__sync.snapshot());
  await expect(page.locator('.sync-settings [role=alert]')).toContainText('系统凭据尚未清理');
  await expect(page.getByLabel('同步账号邮箱')).toHaveValue(connected.email!);
  await expect(page.getByText('自动同步已停止，但账号清理尚未完成。可重试断开，或重新登录原账号；更换账号前必须完成断开。')).toBeVisible();
  await page.getByRole('button', { name: '重试断开', exact: true }).click();
  const confirmation = page.getByRole('group', { name: '确认断开同步', exact: true });
  await expect(confirmation).toContainText('保留本机任务');
  await page.evaluate(() => { (window as any).__sync.failDisconnect = true; });
  await confirmation.getByRole('button', { name: '确认断开', exact: true }).click();
  await expect(page.locator('.sync-operation-error')).toContainText('合成凭据清理失败');
  await expect(page.getByRole('button', { name: '重试断开', exact: true })).toBeEnabled();
  await page.evaluate(() => { (window as any).__sync.failDisconnect = false; });
  await confirmation.getByRole('button', { name: '确认断开', exact: true }).click();
  await expect(page.getByRole('button', { name: '重试断开', exact: true })).toHaveCount(0);
  await expect(page.getByLabel('同步账号邮箱')).toHaveValue('');
  expect(await calls(page, 'sync_sign_out')).toHaveLength(2);
  expect(await page.evaluate(() => (window as any).__sync.snapshot())).toEqual(before);
});

for (const style of ['paper', 'studio', 'editorial', 'mono'] as const) {
  test(`${style} 小尺寸设置可滚动操作同步字段与冲突，不被保存栏遮挡`, async ({ page }) => {
    await page.setViewportSize({ width: 720, height: 520 });
    await nativeSync(page, disabled, style);
    await fillConfig(page);
    const merge = page.getByLabel('将本机任务与此账号合并', { exact: true });
    await merge.check();
    await page.getByRole('button', { name: '登录并启用同步', exact: true }).click();
    await expect(page.locator('.sync-summary')).toContainText(connected.email!);
    await page.evaluate(status => (window as any).__sync.update(status), conflicts);
    const items = page.locator('.sync-conflict');
    for (let i = 0; i < 2; i++) await items.nth(i).getByRole('radio', { name: /保留云端/ }).check();
    const resolve = page.getByRole('button', { name: '确认解决全部冲突', exact: true });
    await resolve.scrollIntoViewIfNeeded();
    const geometry = await resolve.evaluate(element => {
      const box = element.getBoundingClientRect();
      const save = document.querySelector('.settings-save')!.getBoundingClientRect();
      return { bottom: box.bottom, saveTop: save.top, visible: element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2)) };
    });
    expect(geometry.visible).toBe(true);
    expect(geometry.bottom).toBeLessThanOrEqual(geometry.saveTop);
    await resolve.click();
    await expect(page.locator('.sync-state')).toContainText('已同步');
    expect(await page.locator('.sync-settings').evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
  });
}
