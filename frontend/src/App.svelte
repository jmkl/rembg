<script lang="ts">
  import { onMount, tick } from "svelte";
  import {
    SvelteFlow,
    Background,
    Controls,
    type Connection,
    BackgroundVariant,
  } from "@xyflow/svelte";
  import { graph } from "./core/graph.svelte";
  import { defs } from "./core/nodes";
  import { paint } from "./core/paint";
  import { handleHotkeys, isEditable, type Hotkey } from "./core/hotkeys";
  import {
    deleteWorkspace,
    listWorkspaces,
    loadWorkspace,
    renameWorkspace,
    saveWorkspace,
    type WorkspaceKind,
    type WorkspaceMeta,
  } from "./core/workspace";
  import { clearSession, loadSession, saveSession } from "./core/session";
  import { listFiles } from "./core/api";
  import NodeView from "./NodeView.svelte";
  import EditMode from "./EditMode.svelte";
  import FlowFit from "./FlowFit.svelte";

  const nodeTypes = Object.fromEntries(
    Object.keys(defs).map((t) => [t, NodeView]),
  );
  const creatable = Object.values(defs).filter((d) => d.creatable);
  type FolderFile = {
    name: string;
    kind: "input" | "output";
    url: string;
    size: number;
    mtime: number;
  };

  let picker: HTMLInputElement;
  let mainEl: HTMLElement;
  let nodeEl = $state<HTMLElement>();
  let menu = $state<{ x: number; y: number } | null>(null);
  let menuEl = $state<HTMLDivElement>();
  let menuPoint = { x: 0, y: 0 };
  let menuFilter = $state("");
  let filterEl = $state<HTMLInputElement>();
  // Last mouse position, so Shift+A can spawn the menu under the cursor.
  let cursor = { x: -1, y: -1 };

  const filteredCreatable = $derived.by(() => {
    const query = menuFilter.trim().toLowerCase();
    if (!query) return creatable;
    return creatable.filter((d) =>
      `${d.title} ${d.type}`.toLowerCase().includes(query),
    );
  });
  let templateMenu = $state(false);
  let projectMenu = $state(false);
  let templates = $state<WorkspaceMeta[]>([]);
  let projects = $state<WorkspaceMeta[]>([]);
  let currentTemplate = $state<{ id: string; name: string } | null>(null);
  let currentProject = $state<{ id: string; name: string } | null>(null);
  let workspaceBusy = $state(false);
  let files = $state<{ input: FolderFile[]; output: FolderFile[] }>({
    input: [],
    output: [],
  });
  const clampPct = (v: number) => Math.max(20, Math.min(80, v));

  function savedPct(key: string) {
    try {
      const v = Number(globalThis.localStorage?.getItem(key) ?? 50);
      return v >= 20 && v <= 80 ? v : 50;
    } catch {
      return 50;
    }
  }

  // "split" resizes left/right; "splith" resizes the editor above the graph.
  let splitPct = $state(savedPct("rmbg-split"));
  let splitHPct = $state(savedPct("rmbg-split-h"));
  let draggingSplit = $state(false);

  function nodeFlex() {
    if (graph.mode === "split") return `0 0 ${splitPct}%`;
    return "1 1 0";
  }

  /** In stacked mode the editor keeps the top share; the graph fills the rest. */
  function editFlex() {
    return graph.mode === "splith" ? `0 0 ${splitHPct}%` : "1 1 0";
  }

  function splitStart(e: PointerEvent) {
    draggingSplit = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function splitMove(e: PointerEvent) {
    if (!draggingSplit || !mainEl) return;
    const r = mainEl.getBoundingClientRect();
    if (graph.mode === "splith") {
      splitHPct = clampPct(((e.clientY - r.top) / r.height) * 100);
    } else {
      splitPct = clampPct(((e.clientX - r.left) / r.width) * 100);
    }
  }

  function splitEnd(e: PointerEvent) {
    if (!draggingSplit) return;
    draggingSplit = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      // Pointer already released.
    }
    const horizontal = graph.mode === "splith";
    try {
      globalThis.localStorage?.setItem(
        horizontal ? "rmbg-split-h" : "rmbg-split",
        String(horizontal ? splitHPct : splitPct),
      );
    } catch {
      // Storage can be unavailable in private windows.
    }
  }

  function splitReset() {
    if (graph.mode === "splith") {
      splitHPct = 50;
      try {
        globalThis.localStorage?.setItem("rmbg-split-h", "50");
      } catch {
        // Storage can be unavailable in private windows.
      }
    } else {
      splitPct = 50;
      try {
        globalThis.localStorage?.setItem("rmbg-split", "50");
      } catch {
        // Storage can be unavailable in private windows.
      }
    }
  }

  let cut = $state<{
    x1: number;
    y1: number;
    x2: number;
    y2: number;
  } | null>(null);

  /** Squared distance from a point to a line segment. */
  function pointSegDist2(
    px: number,
    py: number,
    ax: number,
    ay: number,
    bx: number,
    by: number,
  ) {
    const dx = bx - ax;
    const dy = by - ay;
    const len2 = dx * dx + dy * dy;
    const t =
      len2 === 0
        ? 0
        : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / len2));
    const qx = ax + t * dx;
    const qy = ay + t * dy;
    return (px - qx) ** 2 + (py - qy) ** 2;
  }

  /** Remove every link whose rendered path crosses the Ctrl+drag cut line. */
  function cutEdges(x1: number, y1: number, x2: number, y2: number) {
    if (!nodeEl) return;
    const hit = new Set<string>();
    const tol2 = 6 * 6;
    for (const g of nodeEl.querySelectorAll<SVGGElement>(
      ".svelte-flow__edge[data-id]",
    )) {
      const id = g.getAttribute("data-id");
      const path = g.querySelector<SVGPathElement>(".svelte-flow__edge-path");
      if (!id || !path) continue;
      try {
        const len = path.getTotalLength();
        const m = path.getScreenCTM();
        if (!len || !m) continue;
        const steps = Math.min(400, Math.max(12, Math.ceil(len / 6)));
        for (let i = 0; i <= steps; i++) {
          const p = path.getPointAtLength((len * i) / steps);
          const sx = m.a * p.x + m.c * p.y + m.e;
          const sy = m.b * p.x + m.d * p.y + m.f;
          if (pointSegDist2(sx, sy, x1, y1, x2, y2) <= tol2) {
            hit.add(id);
            break;
          }
        }
      } catch {
        // The edge may have been removed between the query and the sample.
      }
    }
    if (hit.size) {
      graph.edges = graph.edges.filter((e) => !hit.has(e.id));
    }
  }

  onMount(() => {
    function onDown(e: PointerEvent) {
      if (!e.ctrlKey || e.button !== 0 || !nodeEl || !graph.edges.length)
        return;
      const target = e.target;
      if (!(target instanceof Element) || !nodeEl.contains(target)) return;
      if (
        target.closest(
          ".svelte-flow__node, .svelte-flow__handle, .svelte-flow__controls, .svelte-flow__panel, .svelte-flow__attribution, button, input, textarea, select",
        )
      ) {
        return;
      }
      cut = { x1: e.clientX, y1: e.clientY, x2: e.clientX, y2: e.clientY };
      e.preventDefault();
      e.stopPropagation();
    }

    function onMove(e: PointerEvent) {
      if (!cut) return;
      cut.x2 = e.clientX;
      cut.y2 = e.clientY;
      cutEdges(cut.x1, cut.y1, cut.x2, cut.y2);
      e.preventDefault();
      e.stopPropagation();
    }

    function onUp(e: PointerEvent) {
      if (!cut) return;
      cutEdges(cut.x1, cut.y1, e.clientX, e.clientY);
      cut = null;
      e.preventDefault();
      e.stopPropagation();
    }

    const onCancel = () => (cut = null);
    const onBlur = () => (cut = null);

    window.addEventListener("pointerdown", onDown, true);
    window.addEventListener("pointermove", onMove, true);
    window.addEventListener("pointerup", onUp, true);
    window.addEventListener("pointercancel", onCancel, true);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("pointerdown", onDown, true);
      window.removeEventListener("pointermove", onMove, true);
      window.removeEventListener("pointerup", onUp, true);
      window.removeEventListener("pointercancel", onCancel, true);
      window.removeEventListener("blur", onBlur);
    };
  });

  async function loadFiles() {
    try {
      files = await listFiles();
    } catch {
      // Older backend or offline: just show the usual add menu.
    }
  }

  /** Screen point -> flow coordinates (accounts for pan/zoom). */
  function flowPosition(clientX: number, clientY: number) {
    const viewport = document.querySelector<HTMLElement>(
      ".svelte-flow__viewport",
    );
    const rect = nodeEl?.getBoundingClientRect();
    const m = viewport
      ? new DOMMatrix(getComputedStyle(viewport).transform)
      : new DOMMatrix();
    return {
      x: (clientX - (rect?.left ?? 0) - m.e) / (m.a || 1),
      y: (clientY - (rect?.top ?? 0) - m.f) / (m.d || 1),
    };
  }

  function menuButtons() {
    return menuEl
      ? [
          ...menuEl.querySelectorAll<HTMLButtonElement>(
            "button[data-node-type]",
          ),
        ]
      : [];
  }

  function filterKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      e.stopPropagation();
      focusMenuItem(e.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      const first = filteredCreatable[0];
      if (first) addAt(first.type);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
    }
  }

  function closeMenu() {
    menu = null;
  }

  function closeAllMenus() {
    closeMenu();
    templateMenu = false;
    projectMenu = false;
  }

  /** A socket takes one link, so a new connection replaces the old one. */
  function beforeConnect(connection: Connection): Connection | false {
    const source = connection.source;
    const target = connection.target;
    if (source && target) {
      if (graph.wouldCycle(source, target)) return false;
      const handle = connection.targetHandle ?? "in";
      graph.clearDuplicate(source, target, handle);
      if (!graph.inputIsMulti(target, handle)) {
        graph.clearInput(target, handle);
      }
      if (graph.outputIsSingle(source)) graph.clearOutput(source);
    }
    return connection;
  }

  function toggleTemplates(e: MouseEvent) {
    e.stopPropagation();
    templateMenu = !templateMenu;
    projectMenu = false;
    closeMenu();
    if (templateMenu) void refreshTemplates();
  }

  function toggleProjects(e: MouseEvent) {
    e.stopPropagation();
    projectMenu = !projectMenu;
    templateMenu = false;
    closeMenu();
    if (projectMenu) void refreshProjects();
  }

  async function refreshTemplates() {
    try {
      templates = await listWorkspaces("templates");
    } catch (e) {
      graph.toast((e as Error).message);
    }
  }

  async function refreshProjects() {
    try {
      projects = await listWorkspaces("projects");
    } catch (e) {
      graph.toast((e as Error).message);
    }
  }

  function canReplace() {
    return (
      graph.nodes.length <= 1 ||
      confirm(
        "Replace the current graph with this saved workspace? Unsaved changes will be lost.",
      )
    );
  }

  async function saveTemplate() {
    if (workspaceBusy) return;
    templateMenu = false;
    const suggested = currentTemplate
      ? `${currentTemplate.name} copy`
      : `Template ${templates.length + 1}`;
    const name = prompt("Save the current graph as a template:", suggested);
    if (!name?.trim()) return;
    workspaceBusy = true;
    graph.toast("Saving template…");
    const savedSig = sessionSignature();
    try {
      const item = await saveWorkspace("templates", name.trim());
      currentTemplate = { id: item.id, name: item.name };
      await markSavedBaseline(savedSig);
      await refreshTemplates();
      graph.toast(`Saved template “${item.name}”.`);
    } catch (e) {
      graph.toast((e as Error).message);
    } finally {
      workspaceBusy = false;
    }
  }

  async function loadTemplateItem(item: WorkspaceMeta) {
    if (workspaceBusy) return;
    templateMenu = false;
    if (!canReplace()) return;
    workspaceBusy = true;
    graph.toast(`Loading template “${item.name}”…`);
    try {
      await loadWorkspace("templates", item.id);
      currentTemplate = { id: item.id, name: item.name };
      await markSavedBaseline();
      graph.toast(`Loaded template “${item.name}”.`);
    } catch (e) {
      graph.toast((e as Error).message);
    } finally {
      workspaceBusy = false;
    }
  }

  async function renameItem(kind: WorkspaceKind, item: WorkspaceMeta) {
    if (workspaceBusy) return;
    const label = kind === "templates" ? "template" : "project";
    const name = prompt(`Rename ${label}:`, item.name);
    if (!name?.trim() || name.trim() === item.name) return;
    workspaceBusy = true;
    try {
      const updated = await renameWorkspace(kind, item.id, name.trim());
      if (kind === "templates") {
        if (currentTemplate?.id === item.id) {
          currentTemplate = { id: updated.id, name: updated.name };
        }
        await refreshTemplates();
      } else {
        if (currentProject?.id === item.id) {
          currentProject = { id: updated.id, name: updated.name };
        }
        await refreshProjects();
      }
      graph.toast(`Renamed ${label}.`);
    } catch (e) {
      graph.toast((e as Error).message);
    } finally {
      workspaceBusy = false;
    }
  }

  async function deleteItem(kind: WorkspaceKind, item: WorkspaceMeta) {
    if (workspaceBusy) return;
    const label = kind === "templates" ? "template" : "project";
    if (
      !confirm(
        `Delete ${label} “${item.name}”? Its saved images and processed results are removed too.`,
      )
    ) {
      return;
    }
    workspaceBusy = true;
    try {
      await deleteWorkspace(kind, item.id);
      if (kind === "templates" && currentTemplate?.id === item.id) {
        currentTemplate = null;
        sessionSavedSig = "";
      }
      if (kind === "projects" && currentProject?.id === item.id) {
        currentProject = null;
        sessionSavedSig = "";
      }
      if (kind === "templates") await refreshTemplates();
      else await refreshProjects();
      graph.toast(`Deleted ${label}.`);
    } catch (e) {
      graph.toast((e as Error).message);
    } finally {
      workspaceBusy = false;
    }
  }

  async function saveProject(asNew = false) {
    if (workspaceBusy) return;
    projectMenu = false;
    closeMenu();
    if (!asNew && currentProject) {
      workspaceBusy = true;
      graph.toast("Saving project…");
      const savedSig = sessionSignature();
      try {
        const item = await saveWorkspace(
          "projects",
          currentProject.name,
          currentProject.id,
        );
        currentProject = { id: item.id, name: item.name };
        await markSavedBaseline(savedSig);
        await refreshProjects();
        graph.toast("Project saved.");
      } catch (e) {
        graph.toast((e as Error).message);
      } finally {
        workspaceBusy = false;
      }
      return;
    }

    const suggested = currentProject
      ? `${currentProject.name} copy`
      : `Project ${projects.length + 1}`;
    const name = prompt("Save the current graph as a project:", suggested);
    if (!name?.trim()) return;
    workspaceBusy = true;
    graph.toast("Saving project…");
    const savedSig = sessionSignature();
    try {
      const item = await saveWorkspace("projects", name.trim());
      currentProject = { id: item.id, name: item.name };
      await markSavedBaseline(savedSig);
      await refreshProjects();
      graph.toast(`Saved project “${item.name}”.`);
    } catch (e) {
      graph.toast((e as Error).message);
    } finally {
      workspaceBusy = false;
    }
  }

  async function loadProjectItem(item: WorkspaceMeta) {
    if (workspaceBusy) return;
    projectMenu = false;
    if (!canReplace()) return;
    workspaceBusy = true;
    graph.toast(`Opening project “${item.name}”…`);
    try {
      await loadWorkspace("projects", item.id);
      currentProject = { id: item.id, name: item.name };
      currentTemplate = null;
      await markSavedBaseline();
      graph.toast(`Opened project “${item.name}”.`);
    } catch (e) {
      graph.toast((e as Error).message);
    } finally {
      workspaceBusy = false;
    }
  }

  let sessionReady = false;
  let sessionSavedSig = "";
  let sessionSaving = false;
  let sessionWarned = false;

  /** Cheap-enough state fingerprint used to skip redundant autosaves. */
  function sessionSignature() {
    return JSON.stringify({
      nodes: graph.nodes.map((n) => [
        n.id,
        n.type,
        n.position.x,
        n.position.y,
        n.data,
      ]),
      edges: graph.edges.map((e) => [
        e.id,
        e.source,
        e.target,
        e.sourceHandle,
        e.targetHandle,
      ]),
      params: graph.params,
      assets: Object.keys(graph.assets),
      dataRev: graph.dataRev,
    });
  }

  /** Mark the given graph state as saved and drop the crash-recovery draft. */
  async function markSavedBaseline(sig = sessionSignature()) {
    sessionSavedSig = sig;
    try {
      await clearSession();
    } catch (e) {
      console.warn("Could not clear the saved session", e);
    }
  }

  async function autosaveSession() {
    if (!sessionReady || sessionSaving) return;
    const sig = sessionSignature();
    if (sig === sessionSavedSig) return;
    sessionSaving = true;
    const identity = {
      project: currentProject ? { ...currentProject } : null,
      template: currentTemplate ? { ...currentTemplate } : null,
    };
    try {
      await saveSession(identity);
      // If the user changed something while the write was in flight, leave the
      // old signature so the next tick saves the newer state too.
      if (sessionSignature() === sig) sessionSavedSig = sig;
    } catch (e) {
      console.warn("Could not autosave the session", e);
      if (!sessionWarned) {
        sessionWarned = true;
        graph.toast("Could not autosave this session.");
      }
    } finally {
      sessionSaving = false;
    }
  }

  // Recover the last draft even if the backend died or the page reloaded.
  // Everything is decoded from IndexedDB, so no model or server call is made.
  onMount(() => {
    let cancelled = false;
    void (async () => {
      try {
        const restored = await loadSession();
        if (cancelled) return;
        if (restored) {
          currentProject = restored.project;
          currentTemplate = restored.template;
          graph.toast("Restored unsaved session.");
        }
      } catch (e) {
        console.warn("Could not restore the saved session", e);
        if (!cancelled) graph.toast("Could not restore the unsaved session.");
        void clearSession().catch(() => {});
      } finally {
        if (!cancelled) {
          sessionReady = true;
          sessionSavedSig = sessionSignature();
        }
      }
    })();

    const timer = window.setInterval(() => void autosaveSession(), 1500);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  });

  function focusMenuItem(delta: number) {
    const buttons = menuButtons();
    if (!buttons.length) return;
    const active = document.activeElement;
    let index =
      active instanceof HTMLButtonElement ? buttons.indexOf(active) : -1;
    index =
      index < 0
        ? delta > 0
          ? 0
          : buttons.length - 1
        : (index + delta + buttons.length) % buttons.length;
    buttons[index].focus();
    buttons[index].scrollIntoView({ block: "nearest" });
  }

  function menuKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      e.stopPropagation();
      focusMenuItem(e.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      e.stopPropagation();
      const buttons = menuButtons();
      const target = e.key === "Home" ? buttons[0] : buttons.at(-1);
      target?.focus();
      target?.scrollIntoView({ block: "nearest" });
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeMenu();
    }
  }

  function clampMenu() {
    if (!menu || !menuEl) return;
    const box = menuEl.getBoundingClientRect();
    menu = {
      x: Math.max(8, Math.min(menu.x, window.innerWidth - box.width - 8)),
      y: Math.max(8, Math.min(menu.y, window.innerHeight - box.height - 8)),
    };
  }

  async function openMenuAt(
    x: number,
    y: number,
    opts: { point?: { x: number; y: number } } = {},
  ) {
    templateMenu = false;
    projectMenu = false;
    menuFilter = "";
    menu = { x, y };
    menuPoint = opts.point ?? { x, y };
    void loadFiles();
    await tick();
    clampMenu();
    await tick();
    // The filter is the menu's first stop, so typing immediately narrows it.
    filterEl?.focus();
  }

  /** Shift+A: open the add menu at the cursor, with the filter focused. */
  async function openAddMenu() {
    if (graph.mode === "edit") {
      graph.mode = "splith";
      await tick();
    }
    const r = nodeEl?.getBoundingClientRect();
    const fallback = r
      ? { x: r.left + r.width / 2, y: r.top + r.height / 2 }
      : { x: window.innerWidth / 2, y: window.innerHeight / 2 };
    const point =
      cursor.x >= 0 && cursor.y >= 0 ? { x: cursor.x, y: cursor.y } : fallback;
    await openMenuAt(point.x, point.y, { point });
  }

  function addAt(type: string) {
    if (!menu) return;
    graph.add(type, {}, flowPosition(menuPoint.x, menuPoint.y));
    closeMenu();
  }

  function selectedImage() {
    const node = graph.nodes.find((n) => n.id === graph.selected);
    return node?.type === "image" ? node.id : "";
  }

  function imageNodeAt(target: EventTarget | null) {
    const el =
      target instanceof Element
        ? target.closest<HTMLElement>(".svelte-flow__node-image")
        : null;
    const id = el?.getAttribute("data-id") ?? "";
    return id && graph.nodes.find((n) => n.id === id)?.type === "image"
      ? id
      : "";
  }

  function onpaste(e: ClipboardEvent) {
    if (isEditable(e.target)) return;
    const files = [...(e.clipboardData?.items ?? [])]
      .filter((i) => i.kind === "file" && i.type.startsWith("image/"))
      .map((i) => i.getAsFile()!);
    if (files.length) {
      e.preventDefault();
      const target = selectedImage();
      if (target) {
        const [first, ...rest] = files;
        void graph.replaceImage(target, first);
        if (rest.length) void graph.addFiles(rest);
      } else {
        void graph.addFiles(files);
      }
      return;
    }
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (/^https?:\/\//i.test(text.trim())) {
      e.preventDefault();
      const target = selectedImage();
      if (target) void graph.replaceUrl(target, text);
      else void graph.addUrl(text);
    }
  }

  const external = (e: DragEvent) =>
    [...(e.dataTransfer?.types ?? [])].some(
      (t) => t === "Files" || t === "text/uri-list",
    );

  function ondrop(e: DragEvent) {
    if (!external(e)) return;
    e.preventDefault();
    const files = [...(e.dataTransfer?.files ?? [])].filter((f) =>
      f.type.startsWith("image/"),
    );
    const target = imageNodeAt(e.target);
    if (files.length) {
      if (target) {
        const [first, ...rest] = files;
        void graph.replaceImage(target, first);
        if (rest.length) void graph.addFiles(rest);
      } else {
        void graph.addFiles(files);
      }
      return;
    }
    const url = (e.dataTransfer?.getData("text/uri-list") ?? "")
      .split("\n")
      .find((l) => l && !l.startsWith("#"));
    if (url) {
      if (target) void graph.replaceUrl(target, url);
      else void graph.addUrl(url);
    }
  }

  /** Window shortcuts. Add entries here; unspecified modifiers default to false. */
  const hotkeys: Hotkey[] = [
    {
      key: "Escape",
      when: () => !!cut || !!menu || templateMenu || projectMenu,
      run: () => {
        if (cut) cut = null;
        else if (menu) closeMenu();
        else {
          templateMenu = false;
          projectMenu = false;
        }
      },
    },
    {
      key: "s",
      ctrl: true,
      shift: true,
      run: () => void saveProject(true),
    },
    {
      key: "s",
      ctrl: true,
      run: () => void saveProject(false),
    },
    {
      key: "a",
      shift: true,
      run: () => void openAddMenu(),
    },
    {
      key: "e",
      shift: true,
      run: () => {
        closeMenu();
        graph.mode = graph.mode === "node" ? "edit" : "node";
      },
    },
    {
      key: "d",
      shift: true,
      when: () => graph.mode !== "edit" && !!graph.selected,
      run: () => {
        if (!graph.selected) return;
        closeMenu();
        graph.duplicate(graph.selected);
      },
    },
    {
      key: "ArrowDown",
      when: () => !!menu,
      repeat: true,
      run: () => focusMenuItem(1),
    },
    {
      key: "ArrowUp",
      when: () => !!menu,
      repeat: true,
      run: () => focusMenuItem(-1),
    },
    {
      key: "Delete",
      when: () => !!graph.selected && !menu && !templateMenu && !projectMenu,
      run: () => {
        if (graph.selected) graph.remove(graph.selected);
      },
    },
    {
      key: "Backspace",
      // Always claim Backspace outside inputs, even with nothing selected:
      // otherwise some browsers use it for history navigation.
      run: () => {
        if (menu) {
          closeMenu();
          return;
        }
        if (templateMenu || projectMenu) {
          templateMenu = false;
          projectMenu = false;
          return;
        }
        if (graph.selected) graph.remove(graph.selected);
      },
    },
  ];

  function onkeydown(e: KeyboardEvent) {
    handleHotkeys(e, hotkeys);
  }
</script>

<svelte:window
  {onpaste}
  {ondrop}
  {onkeydown}
  onclick={closeAllMenus}
  onpointermove={(e) => (cursor = { x: e.clientX, y: e.clientY })}
  ondragover={(e) => external(e) && e.preventDefault()}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="titlebar"
  onmousedown={(e) => {
    if (
      e.target === e.currentTarget ||
      (e.target as HTMLElement).closest(".titlebar-drag")
    ) {
      window.ipc?.postMessage("drag");
    }
  }}
>
  <div class="titlebar-tools" role="toolbar" aria-label="Workspace">
    <div class="hud-group">
      <button
        class="hud-btn"
        class:on={graph.mode === "node"}
        title="Node graph"
        aria-label="Node graph"
        onclick={() => (graph.mode = "node")}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="6" cy="6" r="2.4"></circle>
          <circle cx="18" cy="12" r="2.4"></circle>
          <circle cx="6" cy="18" r="2.4"></circle>
          <path d="M8.4 7.2 15.6 11M8.4 16.8 15.6 13"></path>
        </svg>
      </button>
      <button
        class="hud-btn"
        class:on={graph.mode === "split"}
        title="Side-by-side split ↔"
        aria-label="Side-by-side split"
        onclick={() => (graph.mode = "split")}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="3" y="4" width="18" height="16" rx="2"></rect>
          <path d="M12 4v16"></path>
        </svg>
      </button>
      <button
        class="hud-btn"
        class:on={graph.mode === "splith"}
        title="Stacked split ↕"
        aria-label="Stacked split"
        onclick={() => (graph.mode = "splith")}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <rect x="3" y="4" width="18" height="16" rx="2"></rect>
          <path d="M3 12h18"></path>
        </svg>
      </button>
      <button
        class="hud-btn"
        class:on={graph.mode === "edit"}
        title="Edit canvas"
        aria-label="Edit canvas"
        onclick={() => (graph.mode = "edit")}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M4 6h9M17 6h3M4 12h2M10 12h10M4 18h7M15 18h5"></path>
          <circle cx="15" cy="6" r="2"></circle>
          <circle cx="8" cy="12" r="2"></circle>
          <circle cx="13" cy="18" r="2"></circle>
        </svg>
      </button>
    </div>
    <span class="hud-sep"></span>
    <div class="menu-wrap">
      <button
        class="hud-btn"
        class:on={templateMenu}
        title="Templates"
        aria-label="Templates"
        onclick={toggleTemplates}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 3 3 8l9 5 9-5-9-5Z"></path>
          <path d="m3 12 9 5 9-5M3 16l9 5 9-5"></path>
        </svg>
      </button>
      {#if templateMenu}
        <div
          class="menu-pop"
          role="menu"
          tabindex="-1"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              templateMenu = false;
            }
          }}
        >
          <div class="menu-head">
            <span>{currentTemplate ? currentTemplate.name : "Templates"}</span>
            <small>source images + processed results</small>
          </div>
          <button
            class="menu-action"
            disabled={workspaceBusy}
            onclick={() => void saveTemplate()}
          >
            ＋ Save current as template…
          </button>
          <div class="ctxsep"></div>
          <div class="ctxlabel">
            Saved templates <span>{templates.length}</span>
          </div>
          {#if !templates.length}
            <div class="menu-empty">No templates saved yet.</div>
          {:else}
            {#each templates as item (item.id)}
              <div
                class="ws-row"
                class:active={currentTemplate?.id === item.id}
              >
                <button
                  class="ws-main"
                  disabled={workspaceBusy}
                  title={item.name}
                  onclick={() => void loadTemplateItem(item)}
                >
                  {#if item.thumb}
                    <img src={item.thumb} alt="" loading="lazy" />
                  {:else}
                    <span class="ws-thumb"></span>
                  {/if}
                  <span class="ws-text">
                    <span class="fname">{item.name}</span>
                    <small>{item.nodes} nodes</small>
                  </span>
                </button>
                <button
                  class="ws-icon"
                  disabled={workspaceBusy}
                  title="Rename template"
                  onclick={() => void renameItem("templates", item)}>✎</button
                >
                <button
                  class="ws-icon danger"
                  disabled={workspaceBusy}
                  title="Delete template"
                  onclick={() => void deleteItem("templates", item)}>×</button
                >
              </div>
            {/each}
          {/if}
          <div class="menu-foot">
            Loading a template replaces the current graph.
          </div>
        </div>
      {/if}
    </div>
    <div class="menu-wrap">
      <button
        class="hud-btn"
        class:on={projectMenu}
        title={currentProject ? `Project: ${currentProject.name}` : "Project"}
        aria-label="Project"
        onclick={toggleProjects}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z"
          ></path>
        </svg>
      </button>
      {#if projectMenu}
        <div
          class="menu-pop"
          role="menu"
          tabindex="-1"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              projectMenu = false;
            }
          }}
        >
          <div class="menu-head">
            <span
              >{currentProject ? currentProject.name : "Unsaved project"}</span
            >
            <small>
              {currentProject
                ? "Ctrl+S saves in place"
                : "Ctrl+S asks for a name"}
            </small>
          </div>
          <button
            class="menu-action"
            disabled={workspaceBusy}
            onclick={() => void saveProject(false)}
          >
            💾 Save project {#if currentProject}<kbd>Ctrl+S</kbd>{/if}
          </button>
          <button
            class="menu-action"
            disabled={workspaceBusy}
            onclick={() => void saveProject(true)}
          >
            💾 Save project as… <kbd>Ctrl+Shift+S</kbd>
          </button>
          <div class="ctxsep"></div>
          <div class="ctxlabel">
            Open project <span>{projects.length}</span>
          </div>
          {#if !projects.length}
            <div class="menu-empty">No saved projects yet.</div>
          {:else}
            {#each projects as item (item.id)}
              <div class="ws-row" class:active={currentProject?.id === item.id}>
                <button
                  class="ws-main"
                  disabled={workspaceBusy}
                  title={item.name}
                  onclick={() => void loadProjectItem(item)}
                >
                  {#if item.thumb}
                    <img src={item.thumb} alt="" loading="lazy" />
                  {:else}
                    <span class="ws-thumb"></span>
                  {/if}
                  <span class="ws-text">
                    <span class="fname">{item.name}</span>
                    <small>{item.nodes} nodes</small>
                  </span>
                </button>
                <button
                  class="ws-icon"
                  disabled={workspaceBusy}
                  title="Rename project"
                  onclick={() => void renameItem("projects", item)}>✎</button
                >
                <button
                  class="ws-icon danger"
                  disabled={workspaceBusy}
                  title="Delete project"
                  onclick={() => void deleteItem("projects", item)}>×</button
                >
              </div>
            {/each}
          {/if}
          <div class="menu-foot">
            Projects keep their own copy of every image and processed result.
          </div>
        </div>
      {/if}
    </div>
    <span style="display: none;" class="hud-sep"></span>
    <button
      class="hud-btn"
      style="display: none;"
      title="Add image"
      aria-label="Add image"
      onclick={() => picker.click()}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="3" y="6" width="16" height="14" rx="2"></rect>
        <circle cx="8" cy="11" r="1.5"></circle>
        <path d="m4 18 4-4 3 3 2-2 4 4"></path>
        <path d="M19 3v5M16.5 5.5h5"></path>
      </svg>
    </button>
    <button
      style="display: none;"
      class="hud-btn primary"
      title="Export PNG"
      aria-label="Export PNG"
      onclick={() => graph.exportPng()}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 3v11"></path>
        <path d="m8 11 4 4 4-4"></path>
        <path d="M5 20h14"></path>
      </svg>
    </button>
    <input
      bind:this={picker}
      type="file"
      accept="image/*"
      multiple
      hidden
      onchange={(e) => {
        graph.addFiles([...(e.currentTarget.files ?? [])]);
        e.currentTarget.value = "";
      }}
    />
  </div>
  <div class="titlebar-drag">
    <span class="titlebar-title">RMBG Studio</span>
  </div>
  <div class="titlebar-controls">
    <button
      onmousedown={(e) => e.stopPropagation()}
      onclick={() => window.ipc?.postMessage("minimize")}
      aria-label="Minimize">─</button
    >
    <button
      onmousedown={(e) => e.stopPropagation()}
      onclick={() => window.ipc?.postMessage("maximize")}
      aria-label="Maximize">□</button
    >
    <button
      onmousedown={(e) => e.stopPropagation()}
      onclick={() => window.ipc?.postMessage("close")}
      aria-label="Close">✕</button
    >
  </div>
</div>

<main bind:this={mainEl} class:stack={graph.mode === "splith"}>
  {#if graph.mode !== "edit"}
    <section
      class="pane node-pane"
      class:cutting={!!cut}
      bind:this={nodeEl}
      style:flex={nodeFlex()}
    >
      <!-- The Viewer output doubles as the canvas backdrop, like Blender's compositor. -->
      {#if graph.mode !== "split" && graph.mode !== "splith"}
        <canvas class="backdrop" use:paint={graph.result.cv}></canvas>
      {/if}
      <SvelteFlow
        bind:nodes={graph.nodes}
        bind:edges={graph.edges}
        {nodeTypes}
        colorMode="dark"
        deleteKey={null}
        onbeforeconnect={beforeConnect}
        onnodeclick={({ node }) => (graph.selected = node.id)}
        onpaneclick={() => (graph.selected = null)}
        onpanecontextmenu={({ event }) => {
          event.preventDefault();
          void openMenuAt(event.clientX, event.clientY);
        }}
        onnodecontextmenu={({ event }) => {
          event.preventDefault();
          void openMenuAt(event.clientX, event.clientY);
        }}
        onedgecontextmenu={({ event }) => {
          event.preventDefault();
          void openMenuAt(event.clientX, event.clientY);
        }}
        onedgeclick={({ edge, event }) => {
          if (event.altKey) {
            graph.edges = graph.edges.filter((e) => e.id !== edge.id);
          }
        }}
        ondelete={({ nodes }) => graph.cleanup(nodes.map((n) => n.id))}
      >
        <Background
          variant={BackgroundVariant.Cross}
          patternColor="transparent"
        />
        <Controls />
        <FlowFit />
      </SvelteFlow>
    </section>
  {/if}

  {#if graph.mode === "split" || graph.mode === "splith"}
    <div
      class="splitter"
      class:vertical={graph.mode === "split"}
      class:horizontal={graph.mode === "splith"}
      class:drag={draggingSplit}
      role="separator"
      aria-orientation={graph.mode === "splith" ? "horizontal" : "vertical"}
      aria-label="Resize node graph and editor"
      title="Drag to resize · double-click to reset"
      onpointerdown={splitStart}
      onpointermove={splitMove}
      onpointerup={splitEnd}
      onpointercancel={splitEnd}
      ondblclick={splitReset}
    ></div>
  {/if}

  {#if graph.mode !== "node"}
    <section class="pane edit-pane" style:flex={editFlex()}>
      <EditMode />
    </section>
  {/if}
</main>

{#if cut}
  <svg class="cut-overlay" aria-hidden="true">
    <line x1={cut.x1} y1={cut.y1} x2={cut.x2} y2={cut.y2} />
  </svg>
{/if}

{#if menu}
  <div
    class="ctxmenu"
    role="menu"
    tabindex="-1"
    bind:this={menuEl}
    style:left="{menu.x}px"
    style:top="{menu.y}px"
    onclick={(e) => e.stopPropagation()}
    oncontextmenu={(e) => e.preventDefault()}
    onkeydown={menuKey}
  >
    <input
      class="ctxfilter"
      bind:this={filterEl}
      bind:value={menuFilter}
      aria-label="Filter nodes"
      placeholder="Filter nodes…"
      spellcheck="false"
      onkeydown={filterKey}
    />

    <button
      onclick={() => {
        closeMenu();
        picker.click();
      }}
    >
      🖼 Add image…
    </button>
    <div class="ctxsep"></div>
    {#each filteredCreatable as d (d.type)}
      <button data-node-type={d.type} onclick={() => addAt(d.type)}>
        <i class="dot" style:background={d.color}></i>
        {d.title}
      </button>
    {:else}
      <div class="menu-empty">No matching nodes.</div>
    {/each}

    {#if !menuFilter.trim() && files.input.length}
      <div class="ctxsep"></div>
      {#if files.input.length}
        <div class="ctxlabel">
          Input folder <span>{files.input.length}</span>
        </div>
        {#each files.input.slice(0, 12) as f (f.url)}
          <button
            title={f.name}
            onclick={() => {
              closeMenu();
              void graph.addLocal(f.url, f.name);
            }}
          >
            <span class="fname">{f.name}</span>
          </button>
        {/each}
      {/if}
    {/if}
  </div>
{/if}

{#if graph.message}<div class="toast">{graph.message}</div>{/if}
