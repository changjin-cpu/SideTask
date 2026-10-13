import { useEffect, useRef, useState } from 'react';
import { Cloud, LoaderCircle, RefreshCw } from 'lucide-react';
import { useDraft } from '../../lib/drafts';
import { getSyncStatus, onSyncChanged, resolveSync, signInSync, signOutSync, syncAvailable, syncNow, validateSyncSignIn, type SyncChoices, type SyncStatus } from '../../lib/sync';
import './sync.css';

const emptyConfig = { projectUrl: '', publishableKey: '', email: '' };
function describe(value: unknown): string {
  if (value === null || value === undefined) return '空值';
  return typeof value === 'string' ? value || '空文本' : JSON.stringify(value, null, 2);
}
function lastSynced(value: string | null): string {
  if (!value) return '尚未完成首次同步';
  const time = new Date(value);
  return Number.isNaN(time.getTime()) ? '暂无可用时间' : time.toLocaleString();
}

export function SyncSettings({ mobile = false }: { mobile?: boolean }) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [loadError, setLoadError] = useState('');
  const [eventError, setEventError] = useState('');
  const [watchVersion, setWatchVersion] = useState(0);
  const [failure, setFailure] = useState('');
  const [working, setWorking] = useState(false);
  const [config, setConfig] = useState(emptyConfig);
  const [password, setPassword] = useState('');
  const [merge, setMerge] = useState(false);
  const [choices, setChoices] = useState<SyncChoices>({});
  const [disconnect, setDisconnect] = useState(false);
  const sequence = useRef(0);
  const live = useRef(true);
  const inflight = useRef(false);
  const touched = useRef(false);
  const baseline = useRef(emptyConfig);
  const passwordInput = useRef<HTMLInputElement>(null);
  const conflictsKey = JSON.stringify(status?.conflicts ?? []);
  const connected = !!status?.enabled && status.phase !== 'signedOut';
  const hasBinding = !!(status?.email || status?.projectUrl);
  const busy = working || status?.phase === 'syncing';
  const dirty = syncAvailable && !connected && (merge || Object.keys(config).some(key => config[key as keyof typeof config] !== baseline.current[key as keyof typeof config]));

  function accept(next: SyncStatus) {
    setStatus(next);
    setLoadError('');
    if (!touched.current) {
      const initial = { projectUrl: next.projectUrl ?? '', publishableKey: '', email: next.email ?? '' };
      baseline.current = initial;
      setConfig(initial);
    }
  }
  async function refresh() {
    const request = ++sequence.current;
    try {
      const next = await getSyncStatus();
      if (live.current && request === sequence.current) accept(next);
    } catch (reason) {
      if (live.current && request === sequence.current) setLoadError(`无法读取同步状态：${String(reason)}`);
    }
  }
  useEffect(() => {
    live.current = true;
    if (!syncAvailable) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void (async () => {
      try {
        const cleanup = await onSyncChanged(() => { if (!disposed) void refresh(); });
        if (disposed) { cleanup(); return; }
        stop = cleanup;
        setEventError('');
      } catch (reason) {
        if (!disposed) setEventError(`无法接收同步状态更新：${String(reason)}`);
      }
      if (!disposed) await refresh();
    })();
    return () => { disposed = true; live.current = false; sequence.current++; stop?.(); };
  }, [watchVersion]);
  useEffect(() => { setChoices({}); }, [conflictsKey]);
  useDraft('sync-setup', {
    dirty,
    save: async () => {
      setFailure('请先在云同步区域完成登录，或选择放弃未保存的连接配置。密码不会随设置保存。');
      return false;
    },
    discard: () => { setConfig(baseline.current); setPassword(''); setMerge(false); setFailure(''); touched.current = false; },
  });

  async function perform(action: () => Promise<SyncStatus>): Promise<boolean> {
    if (inflight.current) return false;
    inflight.current = true; setWorking(true); setFailure('');
    const request = ++sequence.current;
    try {
      const next = await action();
      if (live.current && request === sequence.current) accept(next);
      else if (live.current) await refresh();
      return true;
    } catch (reason) {
      if (live.current) {
        const message = reason instanceof Error ? reason.message : String(reason);
        setFailure(password ? message.replaceAll(password, '••••') : message);
      }
      return false;
    } finally {
      inflight.current = false;
      if (live.current) setWorking(false);
    }
  }
  async function login() {
    if (busy || inflight.current) return;
    const error = validateSyncSignIn({ ...config, password }, merge);
    if (error) { setFailure(error); return; }
    const credentials = { projectUrl: config.projectUrl.trim().replace(/\/$/, ''), publishableKey: config.publishableKey.trim(), email: config.email.trim(), password, mergeLocal: true as const };
    const succeeded = await perform(() => signInSync(credentials));
    if (live.current) {
      setPassword('');
      if (succeeded) { setMerge(false); baseline.current = config; }
      else passwordInput.current?.focus();
    }
  }
  function change(key: keyof typeof config, value: string) { touched.current = true; setConfig(current => ({ ...current, [key]: value })); }
  const allChosen = !!status?.conflicts.length && status.conflicts.every(conflict => choices[conflict.id]);

  return <section className="settings-section sync-settings" aria-labelledby="sync-heading">
    <div className="settings-section-title"><Cloud size={18}/><h2 id="sync-heading">Mac、Windows 与安卓同步</h2></div>
    <p className="sync-description">{mobile ? '使用与电脑相同的 Supabase 项目和同步账号。任务与安排自动同步，外观各自保留。' : '可选连接自己的 Supabase 项目。电脑与安卓使用同一项目和账号；任务与安排自动同步，窗口和外观各自保留。'}</p>
    {!syncAvailable ? <p className="sync-preview">云同步仅在桌面版使用。当前浏览器预览仍只保存在本机。</p> : <>
      {(loadError || eventError) && <div className="sync-load-error" role="status"><span>{loadError || eventError}</span><button type="button" className="text-button" disabled={working} onClick={() => setWatchVersion(version => version + 1)}>重新读取同步状态</button></div>}
      {!status && !loadError && !eventError && <p role="status">正在读取同步状态…</p>}
      {status?.error && <p className="inline-error" role="alert">{status.error}</p>}
      {status && !connected && <div className="sync-connect" role="group" aria-label="连接云同步" onKeyDown={event => {
        if (event.key === 'Enter' && event.target instanceof HTMLInputElement && !event.nativeEvent.isComposing) {
          event.preventDefault(); event.stopPropagation();
          if (event.target.type === 'password') void login();
        }
      }}>
        {status.phase === 'signedOut' && <p className="sync-description">登录已失效，请重新登录原项目与账号。需要更换账号时，请先断开同步。</p>}
        {!status.enabled && hasBinding && <p className="sync-description">自动同步已停止，但账号清理尚未完成。可重试断开，或重新登录原账号；更换账号前必须完成断开。</p>}
        <p className="sync-description">先按同步配置说明初始化项目，并在 Supabase 中创建或确认邮箱账号。这里填写的是应用账号。</p>
        <div className="sync-fields">
          <label>项目地址<input aria-label="同步项目地址" inputMode="url" autoComplete="off" spellCheck={false} value={config.projectUrl} disabled={busy} onChange={event => change('projectUrl', event.target.value)} placeholder="https://项目编号.supabase.co"/></label>
          <label>公开连接 Key<input aria-label="同步公开连接 Key" autoComplete="off" spellCheck={false} value={config.publishableKey} disabled={busy} onChange={event => change('publishableKey', event.target.value)} placeholder="sb_publishable_… 或 anon key"/></label>
          <label>账号邮箱<input aria-label="同步账号邮箱" inputMode="email" autoComplete="username" spellCheck={false} value={config.email} disabled={busy} onChange={event => change('email', event.target.value)}/></label>
          <label>账号密码<input ref={passwordInput} aria-label="同步账号密码" type="password" autoComplete="current-password" value={password} disabled={busy} onChange={event => setPassword(event.target.value)}/></label>
        </div>
        <label className="sync-merge"><input type="checkbox" checked={merge} disabled={busy} onChange={event => setMerge(event.target.checked)}/><span>将本机任务与此账号合并</span></label>
        <p className="sync-description">连接会上传已保存的任务、今日安排与手动顺序。密码不保存在设置中；刷新凭据由系统凭据库保管。</p>
        <div className="sync-actions"><button type="button" className="primary-button" disabled={busy} onClick={() => void login()}>{working ? <LoaderCircle size={14} className="spin"/> : <Cloud size={14}/>}登录并启用同步</button>{hasBinding && <button type="button" className="text-button" disabled={busy} onClick={() => setDisconnect(true)}>{status.enabled ? '断开同步' : '重试断开'}</button>}</div>
      </div>}
      {status && connected && <div className="sync-connected">
        <dl className="sync-summary"><div><dt>同步账号</dt><dd>{status.email ?? '账号状态待确认'}</dd></div><div><dt>项目</dt><dd>{status.projectUrl ?? '项目状态待确认'}</dd></div><div><dt>最近同步</dt><dd>{lastSynced(status.lastSyncedAt)}</dd></div></dl>
        <p className="sync-state" role="status">{status.phase === 'syncing' ? '正在同步…' : status.phase === 'conflict' ? '有内容需要你选择保留哪一份。' : status.pending ? '本机有待上传的更改，联网后会继续同步。' : status.lastSyncedAt ? '已同步，本机任务可离线使用。' : '已连接，等待首次同步。'}</p>
        {!!status.conflicts.length && <div className="sync-conflicts" role="group" aria-label="处理同步冲突"><p>逐项选择要保留的内容。全部选择并确认前，不会覆盖任一版本。</p>{status.conflicts.map(conflict => <fieldset className="sync-conflict" key={conflict.id} disabled={busy}><legend>{conflict.label}</legend><div className="sync-conflict-options">{(['local', 'remote'] as const).map(side => <div className="sync-conflict-option" key={side}><label><span><input type="radio" name={`sync-conflict-${conflict.id}`} value={side} checked={choices[conflict.id] === side} onChange={() => setChoices(current => ({ ...current, [conflict.id]: side }))}/>{side === 'local' ? '保留本机' : '保留云端'}</span></label><pre tabIndex={0} role="region" aria-label={`${side === 'local' ? '本机' : '云端'}完整内容：${conflict.label}`}>{describe(conflict[side])}</pre></div>)}</div></fieldset>)}<button type="button" className="primary-button" disabled={busy || !allChosen} onClick={() => void perform(() => resolveSync(choices))}>确认解决全部冲突</button></div>}
        <div className="sync-actions"><button type="button" className="secondary-button" disabled={busy || !!status.conflicts.length} onClick={() => void perform(syncNow)}><RefreshCw size={14}/>{status.phase === 'error' ? '重试同步' : '立即同步'}</button><button type="button" className="text-button" disabled={busy} onClick={() => setDisconnect(true)}>断开同步</button></div>
      </div>}
      {status && disconnect && <div className="sync-disconnect" role="group" aria-label="确认断开同步"><p>断开后保留本机任务，云端数据也不会删除。{status.pending ? '还有本机更改未上传，可先继续同步。' : '这台设备将停止自动同步。'}</p><div className="sync-actions"><button type="button" className="secondary-button" disabled={busy} onClick={() => void perform(signOutSync).then(done => { if (done && live.current) { setDisconnect(false); setPassword(''); setMerge(false); } })}>确认断开</button><button type="button" className="text-button" disabled={busy} onClick={() => setDisconnect(false)}>继续同步</button></div></div>}
      {failure && <p className="inline-error sync-operation-error" role="alert">{failure}</p>}
    </>}
  </section>;
}
