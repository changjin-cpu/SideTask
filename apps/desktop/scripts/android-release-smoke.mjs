import { execFileSync } from 'node:child_process';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const serial = process.env.ANDROID_SERIAL || 'emulator-5554';
if (!/^emulator-\d+$/.test(serial)) throw new Error('Release smoke only targets an isolated emulator');
const adb = process.env.ANDROID_ADB || path.join(process.env.ANDROID_HOME || '', 'platform-tools/adb.exe');
function android(...args) { return execFileSync(adb, ['-s',serial,...args], { encoding:'utf8', timeout:30000 }); }
if (!android('emu','avd','name').startsWith('SideTask_API35')) throw new Error('Unexpected emulator');
const out = path.resolve('../../artifacts/android');
fs.mkdirSync(out,{recursive:true});
const report = { serial, passed:false, tests:[], coverage:'Signed release APK through Android accessibility and native input. No WebView debugging or cloud account.' };
function dump() { android('shell','uiautomator','dump','/sdcard/sidetask-release-ui.xml'); return android('shell','cat','/sdcard/sidetask-release-ui.xml'); }
function nodes(xml) { return [...xml.matchAll(/<node\s+([^>]+)>?/g)].map(match => Object.fromEntries([...match[1].matchAll(/([\w-]+)="([^"]*)"/g)].map(attr => [attr[1],attr[2]]))); }
async function waitNode(test) {
  const end = Date.now()+30000;
  do { const node=nodes(dump()).find(test); if(node) return node; await new Promise(resolve=>setTimeout(resolve,300)); } while(Date.now()<end);
  throw new Error('Expected release UI was not found');
}
async function tap(text) {
  const node = await waitNode(node=>node.text===text && node.enabled==='true');
  tapNode(node);
}
function tapNode(node) {
  const bounds=node.bounds.match(/\[(\d+),(\d+)\]\[(\d+),(\d+)\]/);
  if (!bounds) throw new Error('Missing UI bounds');
  android('shell','input','tap',String(Math.round((+bounds[1]+ +bounds[3])/2)),String(Math.round((+bounds[2]+ +bounds[4])/2)));
}
try {
  const info=android('shell','dumpsys','package','com.changjin.sidetask');
  assert.match(info,/versionName=0\.2\.0/);
  assert.doesNotMatch(info,/flags=\[[^\]]*DEBUGGABLE/);
  report.tests.push('release version and non-debuggable flag');
  android('shell','am','start','-n','com.changjin.sidetask/.MainActivity');
  await waitNode(node=>node.text==='今日计划');
  await tap('新建任务');
  tapNode(await waitNode(node=>node.class==='android.widget.EditText'));
  android('shell','input','text','Release_emulator_task');
  android('shell','input','keyevent','4');
  await tap('保存任务');
  await waitNode(node=>node.text==='编辑任务：Release_emulator_task');
  await tap('完成：Release_emulator_task');
  await waitNode(node=>node.text==='撤销完成：Release_emulator_task');
  report.tests.push('release native task create and complete');
  android('shell','am','force-stop','com.changjin.sidetask');
  android('shell','am','start','-n','com.changjin.sidetask/.MainActivity');
  await waitNode(node=>node.text==='撤销完成：Release_emulator_task');
  report.tests.push('release task survives process restart');
  await tap('设置');
  await waitNode(node=>node.text==='Mac、Windows 与安卓同步');
  const settings=dump();
  assert.doesNotMatch(settings,/同步服务尚未就绪|无法初始化同步|本地数据暂时无法/);
  report.tests.push('release console settings and sync service initialize');
  report.passed=true;
  console.log(JSON.stringify(report));
} finally {
  fs.writeFileSync(path.join(out,'release-smoke.json'),JSON.stringify(report,null,2));
  fs.writeFileSync(path.join(out,'release-screen.png'),execFileSync(adb,['-s',serial,'exec-out','screencap','-p']));
}
