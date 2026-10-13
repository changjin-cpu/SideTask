import { expect, test } from '@playwright/test';

test.use({ viewport: { width: 393, height: 852 }, hasTouch: true });
test.beforeEach(async ({ page }) => {
  await page.goto('/?surface=mobile');
  await expect(page.getByRole('navigation', { name: '主导航' })).toBeVisible();
});
test('phone CRUD, today removal, trash restore and persisted ordering', async ({ page }) => {
  await page.getByRole('button', { name: '新建任务', exact: true }).click();
  await page.getByRole('textbox', { name: '任务名称', exact: true }).fill('手机新建任务');
  await page.getByRole('textbox', { name: '任务备注' }).fill('离线也要保留的备注');
  await page.getByLabel('截止日期', { exact: true }).fill('2027-01-20');
  await page.getByRole('button', { name: '保存任务', exact: true }).click();
  await expect(page.getByRole('button', { name: '编辑任务：手机新建任务', exact: true })).toBeVisible();
  await page.getByRole('button', { name: '将手机新建任务移出今日' }).click();
  await expect(page.getByRole('button', { name: '编辑任务：手机新建任务', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: '全部', exact: true }).click();
  await page.getByRole('button', { name: '编辑任务：手机新建任务', exact: true }).click();
  await expect(page.getByRole('textbox', { name: '任务备注' })).toHaveValue('离线也要保留的备注');
  await page.getByRole('button', { name: '移入回收站', exact: true }).click();
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('button', { name: /回收站/ }).click();
  await page.getByRole('button', { name: '恢复任务：手机新建任务' }).click();
  await page.getByRole('button', { name: '截止', exact: true }).click();
  await expect(page.getByRole('button', { name: '编辑任务：手机新建任务', exact: true })).toBeVisible();
  await page.reload();
  await page.getByRole('button', { name: '全部', exact: true }).click();
  await expect(page.getByRole('button', { name: '编辑任务：手机新建任务', exact: true })).toBeVisible();
});
test('phone draft survives close and reload, and no horizontal overflow in either orientation', async ({ page }) => {
  await page.getByRole('button', { name: '新建任务', exact: true }).click();
  await page.getByRole('textbox', { name: '任务名称', exact: true }).fill('草稿中文输入');
  await page.getByRole('button', { name: '收起编辑并保留草稿' }).click();
  await page.reload();
  await page.getByRole('button', { name: /继续未保存的新任务/ }).click();
  await expect(page.getByRole('textbox', { name: '任务名称', exact: true })).toHaveValue('草稿中文输入');
  for (const viewport of [{ width: 320, height: 640 }, { width: 852, height: 393 }]) {
    await page.setViewportSize(viewport);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.getByRole('button', { name: '保存任务', exact: true })).toBeVisible();
  }
});
test('phone theme and settings expose cloud setup without desktop window controls', async ({ page }) => {
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByRole('button', { name: '霜序', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-style', 'studio');
  await page.getByRole('button', { name: '深色', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('heading', { name: 'Mac、Windows 与安卓同步' })).toBeVisible();
  await expect(page.getByText('小窗位置', { exact: true })).toHaveCount(0);
});
