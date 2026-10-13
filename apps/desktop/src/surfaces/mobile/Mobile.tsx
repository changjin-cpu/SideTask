import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CalendarDays, CheckCheck, Cloud, Inbox, Plus, Search, Settings2, Sun, Trash2, X } from 'lucide-react';
import { AppStoreProvider, useAppStore } from '../../lib/store';
import { DraftProvider, useDrafts } from '../../lib/drafts';
import { isDesktop } from '../../lib/native';
import { currentTimeZone, indexSnapshot, localDate, selectDeadlines } from '../../lib/domain';
import { Brand, EmptyState, TaskRow } from '../../components/TaskUI';
import { VirtualTaskList } from '../../components/VirtualTaskList';
import { SyncSettings } from '../../features/sync/SyncSettings';
import type { Action, Page, Priority, Task, TaskChanges, UIStyle } from '../../lib/types';
import './mobile.css';

const titles: Record<Page, string> = { today: '今日计划', all: '全部任务', deadlines: '截止日期', completed: '已完成', trash: '回收站', settings: '设置' };
const tabs = [{ page: 'today', icon: Sun }, { page: 'all', icon: Inbox }, { page: 'deadlines', icon: CalendarDays }, { page: 'settings', icon: Settings2 }] as const;
const styles: Array<[UIStyle, string]> = [['paper','纸笺'], ['studio','霜序'], ['editorial','暖刊'], ['mono','极简']];
type Draft = { id: string | null; revision: number; title: string; notes: string; priority: Priority; dueDate: string; dueTime: string; dueTimezone: string | null; today: boolean };
const draftKey = 'sidetask-mobile-draft-v1';
function readDraft(): Draft | null {
  try {
    const value = JSON.parse(localStorage.getItem(draftKey) || 'null') as Draft | null;
    return value && typeof value.title === 'string' && typeof value.notes === 'string' && typeof value.revision === 'number' && (value.id === null || typeof value.id === 'string') && ['high','normal','low'].includes(value.priority) && typeof value.dueDate === 'string' && typeof value.dueTime === 'string' && typeof value.today === 'boolean' ? value : null;
  } catch { return null; }
}
function validDraft(value: unknown): value is Draft {
  if (!value || typeof value !== 'object') return false;
  const v = value as Draft;
  return typeof v.title === 'string' && typeof v.notes === 'string' && Number.isInteger(v.revision) && (v.id === null || typeof v.id === 'string') && ['high','normal','low'].includes(v.priority) && typeof v.dueDate === 'string' && typeof v.dueTime === 'string' && (v.dueTimezone === null || typeof v.dueTimezone === 'string') && typeof v.today === 'boolean';
}
export default function MobileApplication() {
  return <AppStoreProvider><DraftProvider><MobileContent /></DraftProvider></AppStoreProvider>;
}
function MobileContent() {
  const { dirty: connectionDirty, discardAll } = useDrafts();
  const { snapshot, loading, error, busy, mutate, clearError } = useAppStore();
  const [page, setPage] = useState<Page>('today');
  const [query, setQuery] = useState('');
  const [draft, setDraft] = useState<Draft | null>(() => isDesktop ? null : readDraft());
  const [draftReady, setDraftReady] = useState(!isDesktop);
  const [draftSaving, setDraftSaving] = useState(false);
  const draftQueue = useRef<Promise<void>>(Promise.resolve());
  const enqueuedDraft = useRef<string | undefined>(undefined);
  const draftSequence = useRef(0);
  const [editorOpen, setEditorOpen] = useState(false);
  const [pendingPage, setPendingPage] = useState<Page | null>(null);
  const leaveDialog = useRef<HTMLDialogElement>(null);
  const [failure, setFailure] = useState('');
  const [startupFailure, setStartupFailure] = useState('');
  const scroll = useRef<HTMLElement>(null);
  const editor = useRef<HTMLDialogElement>(null);
  const saving = useRef(false);
  const date = localDate();
  useEffect(() => { if (editorOpen) editor.current?.showModal(); }, [editorOpen]);
  useEffect(() => { if (pendingPage) leaveDialog.current?.showModal(); }, [pendingPage]);
  useEffect(() => {
    Object.assign(window, { __sidetaskBack: () => {
      if (editorOpen) { setEditorOpen(false); return true; }
      if (pendingPage) { setPendingPage(null); return true; }
      if (page !== 'today') { navigate('today'); return true; }
      return false;
    } });
    return () => { Reflect.deleteProperty(window, '__sidetaskBack'); };
  }, [editorOpen, page, pendingPage, connectionDirty]);
  useEffect(() => {
    if (!isDesktop) return;
    const active = () => { void invoke('mobile_active', { active: document.visibilityState !== 'hidden' }).catch(() => {}); };
    document.addEventListener('visibilitychange', active);
    window.addEventListener('online', active);
    active();
    void invoke<{ required: boolean; message: string | null }>('get_startup_recovery').then(result => {
      if (result.required) setStartupFailure(result.message || '本地数据暂时无法打开；原数据已保留，请勿卸载。');
    }).catch(() => {});
    return () => { document.removeEventListener('visibilitychange', active); window.removeEventListener('online', active); };
  }, []);
  useEffect(() => {
    if (!isDesktop) return;
    let live = true;
    void invoke<unknown>('mobile_load_draft').then(value => {
      if (value !== null && !validDraft(value)) throw new Error('草稿格式无法读取，原文件已保留。');
      if (live) { enqueuedDraft.current = JSON.stringify(value); setDraft(value); setDraftReady(true); }
    }).catch(reason => { if (live) setFailure(String(reason)); });
    return () => { live = false; };
  }, []);
  function persistDraft(value: Draft | null) {
    const serialized = JSON.stringify(value);
    if (enqueuedDraft.current === serialized) return draftQueue.current;
    enqueuedDraft.current = serialized;
    const sequence = ++draftSequence.current;
    setDraftSaving(true);
    const pending = draftQueue.current.catch(() => {}).then(async () => {
      if (isDesktop) await invoke('mobile_save_draft', { draft: value });
      else if (value) localStorage.setItem(draftKey, serialized);
      else localStorage.removeItem(draftKey);
    });
    draftQueue.current = pending;
    void pending.then(() => {
      if (sequence === draftSequence.current) setDraftSaving(false);
    }, () => {
      if (sequence === draftSequence.current) { enqueuedDraft.current = undefined; setDraftSaving(false); }
      setFailure('草稿尚未暂存成功，请保留此页面并重试。');
    });
    return pending;
  }
  useEffect(() => { if (draftReady && !saving.current) void persistDraft(draft).catch(() => {}); }, [draft, draftReady]);
  const index = useMemo(() => snapshot ? indexSnapshot(snapshot, date) : null, [snapshot, date]);
  const items = useMemo(() => {
    if (!snapshot || !index) return [];
    const list = page === 'today' ? index.today : page === 'deadlines' ? selectDeadlines(snapshot) : page === 'completed' ? index.completed : page === 'trash' ? index.trashed : index.visibleTasks;
    const search = query.trim().toLocaleLowerCase();
    return search ? list.filter(task => `${task.title}\n${task.notes}`.toLocaleLowerCase().includes(search)) : list;
  }, [snapshot, index, page, query]);
  async function act(action: Action) {
    try { await mutate(action); return true; } catch (reason) { setFailure(String(reason)); return false; }
  }
  function navigate(next: Page) {
    if (next === page) return;
    if (connectionDirty) { setPendingPage(next); return; }
    setPage(next); setQuery(''); setFailure('');
  }
  function edit(task?: Task) {
    if (!draftReady) { setFailure('草稿暂时无法打开，请重新打开应用；原草稿已保留。'); return; }
    if (draft) { setEditorOpen(true); return; }
    setDraft(task ? { id: task.id, revision: task.revision, title: task.title, notes: task.notes, priority: task.priority, dueDate: task.dueDate || '', dueTime: task.dueTime || '', dueTimezone: task.dueTimezone || null, today: index?.todayIds.has(task.id) ?? false }
      : { id: null, revision: 0, title: '', notes: '', priority: 'normal', dueDate: '', dueTime: '', dueTimezone: null, today: page === 'today' });
    setFailure(''); setEditorOpen(true);
  }
  async function discardDraft() {
    try { await persistDraft(null); setDraft(null); setEditorOpen(false); setFailure(''); }
    catch { setFailure('草稿无法清除，请重试。'); }
  }
  async function save(event: FormEvent) {
    event.preventDefault();
    if (!draft || !snapshot || saving.current || busy) return;
    if (!draft.title.trim()) { setFailure('请填写任务名称。'); return; }
    saving.current = true;
    const original = snapshot.tasks.find(task => task.id === draft.id);
    const changes: TaskChanges = { title: draft.title.trim(), notes: draft.notes, priority: draft.priority, dueDate: draft.dueDate || null, dueTime: draft.dueDate && draft.dueTime ? draft.dueTime : null };
    try {
      if (!original || draft.dueDate !== (original.dueDate || '') || draft.dueTime !== (original.dueTime || '') || draft.dueTimezone !== (original.dueTimezone || null)) changes.dueTimezone = draft.dueDate ? draft.dueTimezone || (isDesktop ? await invoke<string>('mobile_time_zone') : currentTimeZone()) : null;
      await persistDraft(draft);
      const result = draft.id ? await mutate({ type: 'updateTask', id: draft.id, changes, expectedRevision: draft.revision })
        : await mutate({ type: 'createTask', task: { ...changes, title: changes.title!, notes: changes.notes!, priority: changes.priority!, dueDate: changes.dueDate!, dueTime: changes.dueTime!, addToToday: draft.today }, date });
      // If clearing the draft fails after committing, retain its new identity so
      // retrying cannot create a second task.
      const committed = draft.id ? result.tasks.find(task => task.id === draft.id) : result.tasks.find(task => !snapshot.tasks.some(old => old.id === task.id));
      if (committed) setDraft({ ...draft, id: committed.id, revision: committed.revision });
      await discardDraft();
    } catch (reason) { setFailure(`${String(reason)}。草稿已保留，可稍后重试。`); }
    finally { saving.current = false; }
  }
  const currentTask = draft?.id ? snapshot?.tasks.find(task => task.id === draft.id) : undefined;
  const staleDraft = !!draft?.id && (!currentTask || currentTask.deletedAt != null || currentTask.revision !== draft.revision);
  const reorder = snapshot && !query.trim() && ['today','all','deadlines'].includes(page) ? {
    revision: snapshot.revision, context: `${page}:${date}`, disabled: busy || editorOpen, title: (task: Task) => task.title,
    commit: (taskIds: string[], observedRevision: number) => mutate(page === 'today' ? { type: 'reorderToday', date, taskIds } : { type: 'reorderTasks', scope: page === 'deadlines' ? 'deadlines' : 'all', taskIds }, observedRevision),
  } : undefined;
  if (startupFailure) return <main className="mobile-startup"><Brand/><h1>数据暂时无法打开</h1><p role="alert">{startupFailure}</p><p>原有文件没有清空，请保留应用及数据，联系维护者恢复备份。</p><button onClick={() => location.reload()}>重试打开</button></main>;
  if (loading || !snapshot || !index) return <main className="mobile-startup"><Brand/><p role="status">{error || '正在打开本机任务…'}</p>{error && <button onClick={() => location.reload()}>重新打开</button>}</main>;
  return <div className="mobile-app">
    <header className="mobile-header"><div><span className="mobile-eyebrow">侧笺 · {new Date().toLocaleDateString('zh-CN', { month: 'long', day: 'numeric', weekday: 'short' })}</span><h1>{titles[page]}</h1></div>{page !== 'settings' && <button className="mobile-add" aria-label="新建任务" onClick={() => edit()}><Plus size={23}/></button>}</header>
    {(error || failure) && !editorOpen && <div className="mobile-error" role="alert">{failure || error}<button aria-label="关闭错误提示" onClick={() => { setFailure(''); clearError(); }}><X size={18}/></button></div>}
    {draft && !editorOpen && <button className="mobile-draft" onClick={() => setEditorOpen(true)}>继续未保存的{draft.id ? '编辑' : '新任务'}：{draft.title || '未命名任务'}</button>}
    <main ref={scroll} className="mobile-main" inert={editorOpen}>
      {page === 'settings' ? <>
        <section className="mobile-settings"><h2>任务管理</h2><button onClick={() => navigate('completed')}><CheckCheck size={20}/>已完成<span>{index.completed.length}</span></button><button onClick={() => navigate('trash')}><Trash2 size={20}/>回收站<span>{index.trashed.length}</span></button></section>
        <section className="mobile-settings"><h2>外观</h2><div className="mobile-options" role="group" aria-label="界面风格">{styles.map(([value, label]) => <button key={value} aria-pressed={snapshot.settings.uiStyle === value} disabled={busy} onClick={() => void act({ type: 'updateSettings', changes: { uiStyle: value } })}>{label}</button>)}</div><div className="mobile-options" role="group" aria-label="明暗模式">{(['light','dark','system'] as const).map((value, i) => <button key={value} aria-pressed={snapshot.settings.theme === value} disabled={busy} onClick={() => void act({ type: 'updateSettings', changes: { theme: value } })}>{['浅色','深色','跟随系统'][i]}</button>)}</div></section>
        <SyncSettings mobile/>
        <p className="mobile-footnote"><Cloud size={16}/>任务先保存在本机。打开应用后自动补传，断网仍可使用。</p>
      </> : <>
        <label className="mobile-search"><Search size={18}/><input aria-label="搜索任务" type="search" value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索任务和备注"/></label>
        {page === 'deadlines' && <label className="mobile-sort">排序<select aria-label="截止日期排序" value={snapshot.settings.ddlSort} disabled={busy} onChange={event => void act({ type: 'updateSettings', changes: { ddlSort: event.target.value as 'date' | 'priority' | 'manual' } })}><option value="date">按日期</option><option value="priority">按重要程度</option><option value="manual">手动排序</option></select></label>}
        {!items.length ? <EmptyState title={query ? '没有找到任务' : page === 'today' ? '今天想做些什么？' : '这里还没有任务'} description={query ? '换个关键词试试。' : '点击右上角＋，记下要做的事。'}/> : <VirtualTaskList items={items} scrollRef={scroll} label={titles[page]} resetKey={`${page}:${query}`} estimateSize={86} reorder={reorder} renderItem={task => task.deletedAt != null ? <div className="mobile-trash"><span>{task.title}</span><button disabled={busy} aria-label={`恢复任务：${task.title}`} onClick={() => void act({ type: 'restoreTask', id: task.id, expectedRevision: task.revision })}>恢复</button></div> : <TaskRow task={task} today={index.todayIds.has(task.id)} disabled={busy} onSelect={() => edit(task)} onToggle={() => void act({ type: 'setCompleted', id: task.id, completed: !task.completed, expectedRevision: task.revision })} onPlan={() => void act({ type: 'planTask', id: task.id, planned: true, date })} onRemoveToday={page === 'today' ? () => void act({ type: 'planTask', id: task.id, planned: false, date }) : undefined} /> } />}
        {page === 'today' && !query && !!index.earlier.length && <section className="mobile-earlier"><h2>此前未完成</h2>{index.earlier.map(task => <TaskRow key={task.id} task={task} disabled={busy} onSelect={() => edit(task)} onToggle={() => void act({ type: 'setCompleted', id: task.id, completed: !task.completed, expectedRevision: task.revision })} onPlan={() => void act({ type: 'planTask', id: task.id, planned: true, date })}/>)}</section>}
      </>}
    </main>
    <nav className="mobile-nav" aria-label="主导航" inert={editorOpen}>{tabs.map(({ page: value, icon: Icon }) => <button key={value} aria-current={page === value ? 'page' : undefined} onClick={() => navigate(value)}><Icon size={21}/><span>{value === 'all' ? '全部' : value === 'deadlines' ? '截止' : value === 'today' ? '今日' : '设置'}</span></button>)}</nav>
    {pendingPage && <dialog ref={leaveDialog} className="mobile-leave" aria-labelledby="mobile-leave-title" onCancel={event => { event.preventDefault(); setPendingPage(null); }}><h2 id="mobile-leave-title">连接配置还未保存</h2><p>返回设置继续登录，或放弃这次输入。密码不会暂存。</p><button className="primary-button" onClick={() => setPendingPage(null)}>继续填写</button><button className="secondary-button" onClick={() => { discardAll(); setPage(pendingPage); setQuery(''); setPendingPage(null); }}>放弃输入并离开</button></dialog>}
    {draft && editorOpen && <dialog ref={editor} className="mobile-editor" aria-labelledby="mobile-editor-title" onCancel={event => { event.preventDefault(); setEditorOpen(false); }}>
      <form onSubmit={event => void save(event)}>
        <header><h2 id="mobile-editor-title">{draft.id ? '编辑任务' : '新建任务'}</h2><button type="button" aria-label="收起编辑并保留草稿" onClick={() => setEditorOpen(false)}><X size={23}/></button></header>
        {(failure || staleDraft) && <p role="alert" className="inline-error">{staleDraft ? '任务已更新或移入回收站。草稿已保留，请先核对最新任务。' : failure}</p>}
        <label>任务名称<input aria-label="任务名称" value={draft.title} autoFocus maxLength={500} onChange={event => setDraft({ ...draft, title: event.target.value })}/></label>
        <label>备注<textarea aria-label="任务备注" rows={4} value={draft.notes} onChange={event => setDraft({ ...draft, notes: event.target.value })}/></label>
        <label>重要程度<select aria-label="重要程度" value={draft.priority} onChange={event => setDraft({ ...draft, priority: event.target.value as Priority })}><option value="normal">普通</option><option value="high">高</option><option value="low">低</option></select></label>
        <label>截止日期<input aria-label="截止日期" type="date" value={draft.dueDate} onChange={event => setDraft({ ...draft, dueDate: event.target.value, dueTime: event.target.value ? draft.dueTime : '' })}/></label>
        {draft.dueDate && <label>具体时刻（可不填）<input aria-label="截止时刻" type="time" value={draft.dueTime} onChange={event => setDraft({ ...draft, dueTime: event.target.value })}/></label>}
        {draft.dueDate && <p className="mobile-footnote">截止时区：{draft.dueTimezone || currentTimeZone()}</p>}
        {!draft.id && <label className="mobile-plan"><input type="checkbox" checked={draft.today} onChange={event => setDraft({ ...draft, today: event.target.checked })}/>加入今日计划</label>}
        <p className="mobile-footnote" role="status">{draftSaving ? '正在暂存草稿…' : '草稿已暂存在本机'}</p>
        <footer><button className="primary-button" type="submit" disabled={busy || staleDraft}>保存任务</button><button type="button" className="secondary-button" disabled={busy} onClick={() => void discardDraft()}>放弃草稿</button></footer>
        {draft.id && currentTask && !staleDraft && <button type="button" className="mobile-trash-action" disabled={busy} onClick={() => void act({ type: 'trashTask', id: currentTask.id, expectedRevision: currentTask.revision }).then(done => { if (done) discardDraft(); })}>移入回收站</button>}
      </form>
    </dialog>}
  </div>;
}
