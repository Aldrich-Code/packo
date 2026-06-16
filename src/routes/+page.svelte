<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { message as showDialogMessage, open, save } from "@tauri-apps/plugin-dialog";
  import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    Archive,
    Check,
    ChevronDown,
    ChevronRight,
    Download,
    ExternalLink,
    FileImage,
    Folder,
    FolderOpen,
    Info,
    KeyRound,
    PackagePlus,
    Pause,
    Play,
    CircleCheck,
    CircleX,
    LoaderCircle,
    Search,
    Settings,
    ShieldCheck,
    Trash2,
    X,
  } from "@lucide/svelte";
  import { onMount, tick } from "svelte";

  type View = "home" | "extract" | "compress";
  type FileKind = "archive" | "zip" | "rar" | "word" | "pdf" | "excel" | "image" | "text" | "folder" | "file";
  type ExtractConflictStrategy = "overwrite" | "skip" | "rename";
  type SevenZipCompressMethod = "LZMA2" | "LZMA";

  type FileInfo = {
    path: string;
    name: string;
    kind: FileKind;
    size: number;
    size_label: string;
    modified_label: string;
  };

  type ArchiveEntry = {
    name: string;
    path: string;
    kind: FileKind;
    type_label: string;
    size: number;
    size_label: string;
    modified_label: string;
    crc: string | null;
    method: string | null;
    is_encrypted: boolean;
    is_hidden: boolean;
    is_executable: boolean;
    is_unsafe_path: boolean;
  };

  type ArchiveRow = {
    name: string;
    type: string;
    size: string;
    sizeValue: number;
    modified: string;
    modifiedValue: number;
    kind: FileKind;
    path: string;
    depth: number;
    crc: string | null;
    method: string | null;
    isEncrypted: boolean;
    isHidden: boolean;
    isExecutable: boolean;
    isUnsafePath: boolean;
  };

  type ArchiveTreeNode = ArchiveRow & {
    children: ArchiveTreeNode[];
  };

  type CompressSource = {
    path: string;
    name: string;
    size: string;
    sizeValue: number;
    kind: FileKind;
  };

  type CompressTreeNode = CompressSource & {
    depth: number;
    children: CompressTreeNode[];
  };

  type ArchiveInfo = {
    path: string;
    name: string;
    format: string;
    size: number;
    size_label: string;
    file_count: number;
    folder_count: number;
    created_label: string;
    output_dir: string;
    entries: ArchiveEntry[];
    folders: string[];
    safety: ArchiveSafetySummary;
    properties: ArchiveProperties;
  };

  type ArchiveSplitSummary = {
    is_split: boolean;
    volume_count: number;
    total_size: number;
    total_size_label: string;
    first_volume: string;
  };

  type ArchiveProperties = {
    uncompressed_size: number;
    uncompressed_size_label: string;
    compression_ratio_label: string;
    method_summary: string;
    crc_available: boolean;
    is_encrypted: boolean;
    encrypted_count: number;
    split: ArchiveSplitSummary;
  };

  type ArchiveSafetySummary = {
    unsafe_paths: number;
    hidden_files: number;
    executables: number;
    samples: string[];
  };

  type RecentFile = {
    path: string;
    name: string;
    format: string;
    time: string;
    kind: FileKind;
  };

  type SystemIconRequest = {
    key: string;
    path?: string | null;
    name: string;
    kind: FileKind;
  };

  type SystemIconResult = {
    key: string;
    dataUrl: string | null;
  };

  type OperationResult = {
    output_path: string;
    message: string;
  };

  type OpenWithApplication = {
    name: string;
    path: string;
    isDefault: boolean;
  };

  type ArchiveRenameEntry = {
    from: string;
    to: string;
  };

  type ArchiveReplaceEntry = {
    entryPath: string;
    sourcePath: string;
  };

  type ArchiveAddEntry = {
    sourcePath: string;
    targetDir: string;
  };

  type ArchiveEditDraft = {
    deleteEntries?: string[];
    renameEntries?: ArchiveRenameEntry[];
    addPaths?: string[];
    addEntries?: ArchiveAddEntry[];
    createDirs?: string[];
    replaceEntries?: ArchiveReplaceEntry[];
    outputPath?: string | null;
  };

  type ArchiveFilterKind = "all" | "files" | "folders" | "encrypted" | "hidden" | "executable";

  type ExtractTaskStatus = "running" | "paused" | "canceling" | "completed" | "failed" | "canceled";
  type AppPanel = "recent" | "settings" | null;
  type WorkspaceBootKind = "extract" | "compress" | null;

  type ExtractTaskProgress = {
    task_id: string;
    status: ExtractTaskStatus;
    total: number;
    completed: number;
    total_bytes: number;
    completed_bytes: number;
    current_bytes: number;
    current_total_bytes: number;
    current_item: string;
    output_path: string;
    message: string;
    error: string | null;
  };

  type ArchivePromiseDragProgress = {
    dragId: string;
    status: "running" | "completed" | "failed";
    total: number;
    completed: number;
    currentItem: string;
    message: string;
    error: string | null;
  };

  type ArchiveFileDragCandidate = {
    row: ArchiveRow;
    rows: ArchiveRow[];
    mode: "move" | "promise";
    startX: number;
    startY: number;
    started: boolean;
    promiseStarted: boolean;
  };

  type ArchiveContextMenu = {
    x: number;
    y: number;
    row: ArchiveRow | null;
  };

  type ArchiveTextPrompt =
    | { kind: "rename"; row: ArchiveRow }
    | { kind: "createFolder"; targetDir: string };

  type DestinationPromptKind = "compress" | "extract";
  type DestinationPromptMode = "current" | "custom";
  type DestinationPrompt = {
    kind: DestinationPromptKind;
    title: string;
    description: string;
    currentPath: string;
  };

  type ArchivePasswordAction =
    | { kind: "open"; path: string }
    | {
        kind: "extract";
        entries: string[];
        outputDir: string;
        taskName: string;
        selectionCount: number;
        totalItems: number;
        totalBytes: number;
        conflictStrategy: ExtractConflictStrategy;
      }
    | {
        kind: "batchExtract";
        paths: string[];
        outputDir: string;
        taskName: string;
        selectionCount: number;
        conflictStrategy: ExtractConflictStrategy;
      }
    | { kind: "preview"; row: ArchiveRow }
    | { kind: "openWith"; row: ArchiveRow; applicationPath: string | null }
    | { kind: "openWithSuggestions"; row: ArchiveRow }
    | { kind: "drag"; rows: ArchiveRow[] }
    | { kind: "testArchive" }
    | { kind: "editArchive"; draft: ArchiveEditDraft; label: string };

  type ArchiveColumnKey = "name" | "type" | "size" | "modified";
  type ArchiveSortDirection = "asc" | "desc";
  type ArchiveSortState = {
    column: ArchiveColumnKey;
    direction: ArchiveSortDirection;
  };

  type ArchiveColumnResize = {
    left: ArchiveColumnKey;
    right: ArchiveColumnKey;
    startX: number;
    startLeftWidth: number;
    startRightWidth: number;
  };

  type PackoPersistedState = {
    version: 1;
    recentFiles?: RecentFile[];
    defaultArchiveFormat?: string;
    defaultSaveDirEnabled?: boolean;
    defaultExtractDirEnabled?: boolean;
    defaultSaveDir?: string;
    defaultExtractDir?: string;
    saveDir?: string;
    extractDir?: string;
    skipDsStore?: boolean;
    skipMacosMetadata?: boolean;
    testAfterCompress?: boolean;
    batchCompressQueue?: boolean;
    sevenZipDictionarySizeMb?: number;
    sevenZipSolid?: boolean;
    compressThreads?: number;
    sevenZipMethod?: SevenZipCompressMethod;
    archiveColumnWidths?: Partial<Record<ArchiveColumnKey, number>>;
    archiveSort?: Partial<ArchiveSortState>;
    extractConflictStrategy?: ExtractConflictStrategy;
    permissionGuideCompleted?: boolean;
  };

  type WorkWindowPayload = {
    kind: "extract" | "compress";
    paths: string[];
  };
  type OpenArchiveResult = {
    ok: boolean;
    passwordPrompt?: boolean;
    error?: string;
  };
  type DefaultArchiveOpenerResult = {
    updated: number;
    failed: { contentType: string; status: number }[];
    registeredAppPath?: string | null;
    warning?: string | null;
  };
  type PermissionGuideSlide = {
    title: string;
    description: string;
    image: string;
    imageAlt: string;
    primaryLabel?: string;
  };
  type PackoMenuAction = "openArchive" | "newArchive" | "extractArchive" | "showRecent" | "showSettings";

  const initialWorkspaceBootKind = detectInitialWorkspaceKind();

  let view = $state<View>("home");
  let isWorkspaceWindow = $state(initialWorkspaceBootKind !== null);
  let workspaceBootKind = $state<WorkspaceBootKind>(initialWorkspaceBootKind);
  let packoWindowShown = false;
  let activeAppPanel = $state<AppPanel>(null);
  let permissionGuideOpen = $state(false);
  let permissionGuideCompleted = $state(false);
  let permissionGuideMessage = $state("");
  let permissionGuideStep = $state(0);
  let missingGuideImages = $state<string[]>([]);
  let defaultOpenerSetting = $state(false);
  let defaultArchiveFormat = $state("ZIP");
  let archiveFormat = $state("ZIP");
  let defaultSaveDirEnabled = $state(false);
  let defaultExtractDirEnabled = $state(false);
  let defaultSaveDir = $state("");
  let defaultExtractDir = $state("");
  let compressionLevel = $state(6);
  let compressionPassword = $state("");
  let extractPassword = $state("");
  let extractConflictStrategy = $state<ExtractConflictStrategy>("overwrite");
  let passwordPromptAction = $state<ArchivePasswordAction | null>(null);
  let passwordPromptValue = $state("");
  let passwordPromptMessage = $state("");
  let skipDsStore = $state(false);
  let skipMacosMetadata = $state(false);
  let testAfterCompress = $state(true);
  let batchCompressQueue = $state(false);
  let sevenZipDictionarySizeMb = $state(64);
  let sevenZipSolid = $state(false);
  let compressThreads = $state(2);
  let sevenZipMethod = $state<SevenZipCompressMethod>("LZMA2");
  let splitArchive = $state(false);
  let splitSizeMb = $state(100);
  let homeSearch = $state("");
  let homeSearchMessage = $state("");
  let recentFiles = $state<RecentFile[]>([]);
  let archiveInfo = $state<ArchiveInfo | null>(null);
  let pendingArchivePath = $state("");
  let archiveTree = $state<ArchiveTreeNode[]>([]);
  let archiveSearchQuery = $state("");
  let archiveKindFilter = $state<ArchiveFilterKind>("all");
  let archiveIntegrityMessage = $state("");
  let archiveInfoPanelOpen = $state(false);
  let expandedArchiveFolders = $state<string[]>([]);
  let selectedArchivePaths = $state<string[]>([]);
  let selectedItems = $state<CompressSource[]>([]);
  let compressTree = $state<CompressTreeNode[]>([]);
  let expandedCompressFolders = $state<string[]>([]);
  let selectedCompressPaths = $state<string[]>([]);
  let excludedCompressPaths = $state<string[]>([]);
  let archiveDragSelecting = $state(false);
  let compressDragSelecting = $state(false);
  let archiveDragMode = $state<"select" | "deselect">("select");
  let compressDragMode = $state<"select" | "deselect">("select");
  let archiveColumnWidths = $state<Record<ArchiveColumnKey, number>>({
    name: 220,
    type: 82,
    size: 72,
    modified: 146,
  });
  let archiveSort = $state<ArchiveSortState>({ column: "name", direction: "asc" });
  let archiveColumnResize = $state<ArchiveColumnResize | null>(null);
  let previewCache = $state(new Map<string, string>());
  let systemIconCache = $state(new Map<string, string | null>());
  let archiveContextMenu = $state<ArchiveContextMenu | null>(null);
  let contextOpenWithApps = $state<OpenWithApplication[]>([]);
  let contextOpenWithRowPath = $state("");
  let contextOpenWithLoading = $state(false);
  let archiveFileDragCandidate: ArchiveFileDragCandidate | null = null;
  let archiveFileDragStarted = false;
  let archiveInternalDropTargetPath = $state<string | null>(null);
  let archiveExternalDropTargetPath = $state<string | null>(null);
  let archiveName = $state("");
  let saveDir = $state("");
  let extractDir = $state("");
  let statusMessage = $state("");
  let errorMessage = $state("");
  let busy = $state(false);
  let extractTask = $state<ExtractTaskProgress | null>(null);
  let extractTaskArchiveName = $state("");
  let extractTaskSelectionCount = $state(0);
  let extractTaskMode = $state<"extract" | "preview" | "compress" | "edit">("extract");
  let archivePromiseDragProgress = $state<ArchivePromiseDragProgress | null>(null);
  let previewTaskEntryPath = $state("");
  let handledCompletedTaskId = "";
  let activePasswordRetryAction: ArchivePasswordAction | null = null;
  let extractProgressTimer: number | null = null;
  let archiveEditProgressTimer: number | null = null;
  let archivePromiseDragHideTimer: number | null = null;
  let preferencesLoaded = $state(false);
  let syncingExternalRecentFiles = false;
  let recentSearchInput = $state<HTMLInputElement | null>(null);
  let destinationPrompt = $state<DestinationPrompt | null>(null);
  let destinationPromptMode = $state<DestinationPromptMode>("current");
  let destinationPromptCustomPath = $state("");
  let destinationPromptMessage = $state("");
  let archiveTextPrompt = $state<ArchiveTextPrompt | null>(null);
  let archiveTextPromptValue = $state("");
  let archiveTextPromptMessage = $state("");
  let archiveTextPromptInput = $state<HTMLInputElement | null>(null);
  let destinationPromptResolver: ((path: string | null) => void) | null = null;
  let systemIconRequestTimer: number | null = null;
  const pendingSystemIconRequests = new Map<string, SystemIconRequest>();

  const supportedExtractFormats =
    "ZIP、RAR、7Z、GZ/GZIP、BZ2/BZIP2、XZ、TAR、TGZ、TBZ、TXZ、LZH/LHA、Z、ZSTD、LZMA/LZMA2、LZ4、ISO、分卷压缩包";
  const permissionGuideSlides: PermissionGuideSlide[] = [
    {
      title: "欢迎使用 Packo",
      description: "压缩、解压、预览和编辑压缩包，都围绕文件树和拖拽完成。",
      image: "/onboarding/welcome.png",
      imageAlt: "Packo 首页和拖拽入口示意图",
      primaryLabel: "下一步",
    },
    {
      title: "文件树拖拽操作",
      description: "同一行不同区域有不同拖拽含义，最常用的三个手势都在这里。",
      image: "/onboarding/file-tree-drag-zones.png",
      imageAlt: "文件树图标、名称和其他字段的拖拽热区示意图",
      primaryLabel: "下一步",
    },
    {
      title: "右键编辑和打开方式",
      description: "文件树右键可以直接处理压缩包内容，不需要先完整解压。",
      image: "/onboarding/context-menu.png",
      imageAlt: "压缩包文件树右键菜单示意图",
      primaryLabel: "下一步",
    },
    {
      title: "设置默认打开方式",
      description: "把常见压缩包交给 Packo 打开，之后双击压缩包会直接进入预览窗口。",
      image: "/onboarding/default-open.png",
      imageAlt: "将 Packo 设置为默认打开方式的示意图",
      primaryLabel: "开始使用",
    },
  ];
  const supportedCompressFormats = [
    "ZIP",
    "7Z",
    "TAR",
    "TGZ",
    "TBZ",
    "TXZ",
    "GZ",
    "BZ2",
    "XZ",
    "Z",
    "ZSTD",
    "LZMA",
    "LZMA2",
    "LZ4",
  ];
  const archiveExtensions = [
    "zip",
    "rar",
    "7z",
    "tar",
    "gz",
    "gzip",
    "bz2",
    "bzip2",
    "xz",
    "iso",
    "tgz",
    "tbz",
    "tbz2",
    "txz",
    "lzh",
    "lha",
    "z",
    "zst",
    "zstd",
    "tzst",
    "lzma",
    "lzma2",
    "tlzma",
    "lz4",
    "tlz4",
    "001",
    ...Array.from({ length: 99 }, (_, index) => `z${String(index + 1).padStart(2, "0")}`),
  ];
  const zipContainerExtensionPattern =
    /\.(docx|docm|dotx|dotm|pptx|pptm|potx|potm|ppsx|ppsm|xlsx|xlsm|xltx|xltm|xlam|odt|ods|odp|odg|odf|epub|jar|war|ear|apk|aab|ipa|vsdx|vstx|vssx|vsdm|vstm|vssm|whl|nupkg)$/i;
  const archiveSuffixPattern =
    /\.(part\d+\.rar|(?:zip|rar|7z|tar|tar\.(?:gz|gzip|bz2|bzip2|xz|z|zst|zstd|lzma2|lzma|lz4))\.\d{3}|z\d{2}|tar\.(gz|gzip|bz2|bzip2|xz|z|zst|zstd|lzma2|lzma|lz4)|tgz|tbz2?|txz|tzst|tlzma|tlz4|zip|rar|7z|tar|gz|gzip|bz2|bzip2|xz|iso|lzh|lha|z|zst|zstd|lzma2|lzma|lz4|\d{3}|docx|docm|dotx|dotm|pptx|pptm|potx|potm|ppsx|ppsm|xlsx|xlsm|xltx|xltm|xlam|odt|ods|odp|odg|odf|epub|jar|war|ear|apk|aab|ipa|vsdx|vstx|vssx|vsdm|vstm|vssm|whl|nupkg)$/i;
  const archiveExtensionPattern =
    /\.(zip|rar|7z|tar|gz|gzip|bz2|bzip2|xz|iso|tgz|tbz2?|txz|lzh|lha|z|zst|zstd|tzst|lzma2|lzma|tlzma|lz4|tlz4|\d{3}|z\d{2}|docx|docm|dotx|dotm|pptx|pptm|potx|potm|ppsx|ppsm|xlsx|xlsm|xltx|xltm|xlam|odt|ods|odp|odg|odf|epub|jar|war|ear|apk|aab|ipa|vsdx|vstx|vssx|vsdm|vstm|vssm|whl|nupkg)$/i;
  const finishedExtractStatuses = new Set<ExtractTaskStatus>(["completed", "failed", "canceled"]);
  const packoStorageKey = "packo.preferences.v1";
  const archiveColumnMin: Record<ArchiveColumnKey, number> = {
    name: 80,
    type: 38,
    size: 44,
    modified: 76,
  };
  const archiveColumnMax: Record<ArchiveColumnKey, number> = {
    name: 560,
    type: 180,
    size: 180,
    modified: 320,
  };
  const systemIconBatchSize = 220;

  function detectInitialWorkspaceKind(): WorkspaceBootKind {
    if (typeof window === "undefined") return null;

    const queryWorkspace = new URLSearchParams(window.location.search).get("workspace");
    if (queryWorkspace === "extract" || queryWorkspace === "compress") {
      return queryWorkspace;
    }

    if (!isTauriRuntime()) return null;
    const label = getCurrentWindow().label;
    if (label.startsWith("packo-extract-")) return "extract";
    if (label.startsWith("packo-compress-")) return "compress";
    return null;
  }

  function workspaceBootTitle() {
    return workspaceBootKind === "compress" ? "正在准备压缩窗口" : "正在打开压缩包";
  }

  function workspaceBootDescription() {
    if (workspaceBootKind === "compress") {
      return "正在加载待压缩项目和压缩设置";
    }
    return pendingArchivePath ? `正在读取 ${fileNameFromPath(pendingArchivePath)}` : "正在读取压缩包内容";
  }

  function go(nextView: View) {
    view = nextView;
    activeAppPanel = null;
    archiveInfoPanelOpen = false;
  }

  function isTauriRuntime() {
    return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  }

  async function showPackoWindowWhenReady() {
    if (!isTauriRuntime() || packoWindowShown) return;
    packoWindowShown = true;

    await tick();
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));

    try {
      const currentWindow = getCurrentWindow();
      await currentWindow.show();
      await currentWindow.setFocus();
    } catch {
      packoWindowShown = false;
    }
  }

  async function closeInvalidArchiveWorkspace(path: string, detail?: string) {
    if (!isTauriRuntime()) return;
    const name = fileNameFromPath(path) || "所选文件";
    const detailText = detail ? `\n\n${detail}` : "";
    await showDialogMessage(`“${name}”不是有效的压缩文件，无法解压。${detailText}`, {
      title: "无法打开压缩包",
      kind: "error",
    });
    await getCurrentWindow().close();
  }

  function isMainLauncherWindow() {
    return isTauriRuntime() && !isWorkspaceWindow && view === "home";
  }

  function isLauncherHomeView() {
    return !isWorkspaceWindow && !workspaceBootKind && view === "home";
  }

  function fileNameFromPath(path = "") {
    return path.split(/[\\/]/).pop() || path;
  }

  function systemIconExtension(name: string, path?: string | null) {
    const source = name || fileNameFromPath(path ?? "");
    const extension = source.includes(".") ? source.split(".").pop()?.trim().toLowerCase() : "";
    return extension || "";
  }

  function systemIconRequest(kind: FileKind, name: string, path?: string | null): SystemIconRequest {
    const extension = systemIconExtension(name, path);
    const key = kind === "folder" ? "system:folder" : `system:${extension || kind || "file"}`;
    return {
      key,
      path: path || null,
      name: name || fileNameFromPath(path ?? "") || "file",
      kind,
    };
  }

  function systemIconSource(kind: FileKind, name: string, path?: string | null) {
    return systemIconCache.get(systemIconRequest(kind, name, path).key) ?? null;
  }

  function scheduleSystemIconFlush() {
    if (typeof window === "undefined" || systemIconRequestTimer !== null) return;
    systemIconRequestTimer = window.setTimeout(() => {
      systemIconRequestTimer = null;
      void flushSystemIconRequests();
    }, 40);
  }

  function queueSystemIconRequests(items: SystemIconRequest[]) {
    if (!isTauriRuntime()) return;
    let added = false;
    for (const item of items) {
      if (systemIconCache.has(item.key) || pendingSystemIconRequests.has(item.key)) continue;
      pendingSystemIconRequests.set(item.key, item);
      added = true;
    }
    if (added) scheduleSystemIconFlush();
  }

  async function flushSystemIconRequests() {
    if (pendingSystemIconRequests.size === 0) return;
    const items = Array.from(pendingSystemIconRequests.values()).slice(0, systemIconBatchSize);
    for (const item of items) {
      pendingSystemIconRequests.delete(item.key);
    }

    try {
      const results = await invoke<SystemIconResult[]>("system_file_icons", { items });
      const nextCache = new Map(systemIconCache);
      for (const item of items) {
        nextCache.set(item.key, null);
      }
      for (const result of results) {
        nextCache.set(result.key, result.dataUrl);
      }
      systemIconCache = nextCache;
    } catch {
      const nextCache = new Map(systemIconCache);
      for (const item of items) {
        nextCache.set(item.key, null);
      }
      systemIconCache = nextCache;
    } finally {
      if (pendingSystemIconRequests.size > 0) scheduleSystemIconFlush();
    }
  }

  async function openExtractWorkspace(path: string) {
    if (!isTauriRuntime()) {
      await openArchive(path);
      return;
    }
    await invoke("open_extract_window", { path });
  }

  async function openExtractWorkspaces(paths: string[]) {
    for (const path of paths) {
      addRecentArchivePath(path);
      await openExtractWorkspace(path);
    }
  }

  async function shouldCompressSingleImportPath(path: string) {
    if (!path) return false;
    try {
      const [info] = await invoke<FileInfo[]>("describe_paths", { paths: [path] });
      return info?.kind === "folder";
    } catch {
      return false;
    }
  }

  async function openCompressWorkspace(paths: string[], allowEmpty = false) {
    if (paths.length === 0 && !allowEmpty) return;
    if (!isTauriRuntime()) {
      if (paths.length === 0) {
        resetCompressState();
        go("compress");
        return;
      }
      await addCompressPaths(paths);
      return;
    }
    await invoke("open_compress_window", { paths });
  }

  async function openBlankCompressWorkspace() {
    await openCompressWorkspace([], true);
  }

  function startWindowDrag(event: MouseEvent) {
    if (!isTauriRuntime() || event.button !== 0) return;

    const target = event.target as HTMLElement | null;
    if (target?.closest("button, input, label, [data-no-drag]")) return;

    void getCurrentWindow().startDragging();
  }

  function clearStatus() {
    errorMessage = "";
    statusMessage = "";
    homeSearchMessage = "";
    archiveIntegrityMessage = "";
  }

  function isPlainObject(value: unknown): value is Record<string, unknown> {
    return Boolean(value && typeof value === "object" && !Array.isArray(value));
  }

  function sanitizePersistedRecentFiles(value: unknown) {
    if (!Array.isArray(value)) return [];
    return value
      .filter((item): item is RecentFile => {
        return (
          isPlainObject(item) &&
          typeof item.path === "string" &&
          typeof item.name === "string" &&
          typeof item.format === "string" &&
          typeof item.time === "string"
        );
      })
      .map((item) => ({
        path: item.path,
        name: item.name,
        format: archiveFormatOf(item.path || item.name),
        time: normalizeRecentTimeLabel(item.time),
        kind: "archive" as FileKind,
      }))
      .slice(0, 8);
  }

  function normalizeRecentTimeLabel(value: string) {
    const trimmed = value.trim();
    if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/.test(trimmed)) return trimmed;
    const chineseDate = trimmed.match(/^(\d{4})年(\d{1,2})月(\d{1,2})日\s+(\d{1,2}):(\d{2})$/);
    if (chineseDate) {
      const [, year, month, day, hour, minute] = chineseDate;
      return `${year}-${month.padStart(2, "0")}-${day.padStart(2, "0")} ${hour.padStart(2, "0")}:${minute}`;
    }
    return currentRecentTimeLabel();
  }

  function sanitizeArchiveColumnWidths(value: unknown) {
    if (!isPlainObject(value)) return null;
    const next = { ...archiveColumnWidths };
    for (const column of Object.keys(archiveColumnWidths) as ArchiveColumnKey[]) {
      const width = value[column];
      if (typeof width === "number" && Number.isFinite(width)) {
        next[column] = clampArchiveColumnWidth(column, width);
      }
    }
    return next;
  }

  function isArchiveColumnKey(value: unknown): value is ArchiveColumnKey {
    return value === "name" || value === "type" || value === "size" || value === "modified";
  }

  function sanitizeArchiveSort(value: unknown) {
    if (!isPlainObject(value)) return null;
    if (!isArchiveColumnKey(value.column)) return null;
    const direction = value.direction === "desc" ? "desc" : "asc";
    return { column: value.column, direction } satisfies ArchiveSortState;
  }

  function isExtractConflictStrategy(value: unknown): value is ExtractConflictStrategy {
    return value === "overwrite" || value === "skip" || value === "rename";
  }

  function loadPackoPreferences() {
    if (typeof window === "undefined") return;
    try {
      const raw = window.localStorage.getItem(packoStorageKey);
      if (!raw) return;
      const parsed: unknown = JSON.parse(raw);
      if (!isPlainObject(parsed)) return;

      recentFiles = sanitizePersistedRecentFiles(parsed.recentFiles);
      if (typeof parsed.defaultArchiveFormat === "string" && supportedCompressFormats.includes(parsed.defaultArchiveFormat)) {
        defaultArchiveFormat = parsed.defaultArchiveFormat;
        archiveFormat = defaultArchiveFormat;
        compressionLevel = defaultCompressionLevel(defaultArchiveFormat);
      }
      if (typeof parsed.defaultSaveDirEnabled === "boolean") defaultSaveDirEnabled = parsed.defaultSaveDirEnabled;
      if (typeof parsed.defaultExtractDirEnabled === "boolean") defaultExtractDirEnabled = parsed.defaultExtractDirEnabled;
      if (typeof parsed.defaultSaveDir === "string") {
        defaultSaveDir = parsed.defaultSaveDir;
      } else if (typeof parsed.saveDir === "string") {
        defaultSaveDir = parsed.saveDir;
      }
      if (typeof parsed.defaultExtractDir === "string") {
        defaultExtractDir = parsed.defaultExtractDir;
      } else if (typeof parsed.extractDir === "string") {
        defaultExtractDir = parsed.extractDir;
      }
      if (typeof parsed.skipDsStore === "boolean") skipDsStore = parsed.skipDsStore;
      if (typeof parsed.skipMacosMetadata === "boolean") skipMacosMetadata = parsed.skipMacosMetadata;
      if (typeof parsed.testAfterCompress === "boolean") testAfterCompress = parsed.testAfterCompress;
      if (typeof parsed.batchCompressQueue === "boolean") batchCompressQueue = parsed.batchCompressQueue;
      if (typeof parsed.sevenZipDictionarySizeMb === "number" && Number.isFinite(parsed.sevenZipDictionarySizeMb)) {
        sevenZipDictionarySizeMb = Math.min(1024, Math.max(1, Math.round(parsed.sevenZipDictionarySizeMb)));
      }
      if (typeof parsed.sevenZipSolid === "boolean") sevenZipSolid = parsed.sevenZipSolid;
      if (typeof parsed.compressThreads === "number" && Number.isFinite(parsed.compressThreads)) {
        compressThreads = Math.min(32, Math.max(1, Math.round(parsed.compressThreads)));
      }
      if (parsed.sevenZipMethod === "LZMA" || parsed.sevenZipMethod === "LZMA2") {
        sevenZipMethod = parsed.sevenZipMethod;
      }

      const widths = sanitizeArchiveColumnWidths(parsed.archiveColumnWidths);
      if (widths) archiveColumnWidths = widths;

      const sort = sanitizeArchiveSort(parsed.archiveSort);
      if (sort) archiveSort = sort;
      if (isExtractConflictStrategy(parsed.extractConflictStrategy)) {
        extractConflictStrategy = parsed.extractConflictStrategy;
      }
      if (typeof parsed.permissionGuideCompleted === "boolean") {
        permissionGuideCompleted = parsed.permissionGuideCompleted;
      }
    } catch {
      window.localStorage.removeItem(packoStorageKey);
    }
  }

  function syncRecentFilesFromStorage() {
    if (typeof window === "undefined") return;
    try {
      const raw = window.localStorage.getItem(packoStorageKey);
      if (!raw) return;
      const parsed: unknown = JSON.parse(raw);
      if (!isPlainObject(parsed)) return;
      syncingExternalRecentFiles = true;
      recentFiles = sanitizePersistedRecentFiles(parsed.recentFiles);
      window.setTimeout(() => {
        syncingExternalRecentFiles = false;
      }, 0);
    } catch {
      window.localStorage.removeItem(packoStorageKey);
    }
  }

  function savePackoPreferences() {
    if (!preferencesLoaded || syncingExternalRecentFiles || typeof window === "undefined") return;
    const state: PackoPersistedState = {
      version: 1,
      recentFiles,
      defaultArchiveFormat,
      defaultSaveDirEnabled,
      defaultExtractDirEnabled,
      defaultSaveDir,
      defaultExtractDir,
      skipDsStore,
      skipMacosMetadata,
      testAfterCompress,
      batchCompressQueue,
      sevenZipDictionarySizeMb,
      sevenZipSolid,
      compressThreads,
      sevenZipMethod,
      archiveColumnWidths,
      archiveSort,
      extractConflictStrategy,
      permissionGuideCompleted,
    };
    try {
      window.localStorage.setItem(packoStorageKey, JSON.stringify(state));
    } catch {
      // Ignore storage quota/private mode failures; the app still works for the current session.
    }
  }

  function toggleAppPanel(panel: Exclude<AppPanel, null>) {
    archiveInfoPanelOpen = false;
    activeAppPanel = activeAppPanel === panel ? null : panel;
  }

  function openRecentPanel(focusSearch = false) {
    archiveInfoPanelOpen = false;
    activeAppPanel = "recent";
    if (focusSearch) {
      window.setTimeout(() => {
        recentSearchInput?.focus();
      }, 0);
    }
  }

  function openSettingsPanel() {
    archiveInfoPanelOpen = false;
    activeAppPanel = "settings";
  }

  function closeAppPanel() {
    activeAppPanel = null;
  }

  function showFirstLaunchPermissionGuide() {
    if (permissionGuideCompleted || !isLauncherHomeView()) return;
    window.setTimeout(() => {
      if (!permissionGuideCompleted && isLauncherHomeView()) {
        permissionGuideStep = 0;
        permissionGuideOpen = true;
      }
    }, 260);
  }

  function openPermissionGuide() {
    activeAppPanel = null;
    permissionGuideMessage = "";
    permissionGuideStep = 0;
    permissionGuideOpen = true;
  }

  function closePermissionGuide() {
    permissionGuideOpen = false;
    permissionGuideMessage = "";
    permissionGuideCompleted = true;
  }

  function currentPermissionGuideSlide() {
    return permissionGuideSlides[Math.min(permissionGuideStep, permissionGuideSlides.length - 1)];
  }

  function guideImageMissing(image: string) {
    return missingGuideImages.includes(image);
  }

  function markGuideImageMissing(image: string) {
    if (!missingGuideImages.includes(image)) {
      missingGuideImages = [...missingGuideImages, image];
    }
  }

  function previousPermissionGuideStep() {
    permissionGuideMessage = "";
    permissionGuideStep = Math.max(0, permissionGuideStep - 1);
  }

  function nextPermissionGuideStep() {
    permissionGuideMessage = "";
    if (permissionGuideStep >= permissionGuideSlides.length - 1) {
      closePermissionGuide();
      return;
    }
    permissionGuideStep += 1;
  }

  async function setPackoAsDefaultArchiveOpener() {
    permissionGuideMessage = "";
    if (!isTauriRuntime()) {
      permissionGuideMessage = "在桌面应用中才能设置默认打开方式。";
      return;
    }

    defaultOpenerSetting = true;
    try {
      const result = await invoke<DefaultArchiveOpenerResult>("set_packo_as_default_archive_opener");
      const failedCount = result.failed.length;
      const registeredText = result.registeredAppPath ? `已注册 ${fileNameFromPath(result.registeredAppPath)}。` : "";
      const warningText = result.warning ? ` ${result.warning}` : "";
      permissionGuideMessage =
        failedCount > 0
          ? `已将 Packo 设为 ${result.updated} 类压缩包的默认打开方式，${failedCount} 类未能写入。${registeredText}${warningText}`
          : `已将 Packo 设为 ${result.updated} 类常见压缩包的默认打开方式。${registeredText}${warningText}`;
    } catch (error) {
      permissionGuideMessage = `${String(error)} 如果 Finder 未立即更新，请重新打开“显示简介”窗口或重启 Finder。`;
    } finally {
      defaultOpenerSetting = false;
    }
  }

  function handleGlobalKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented) return;
    const key = event.key.toLowerCase();

    if (key === "escape") {
      if (permissionGuideOpen) {
        event.preventDefault();
        closePermissionGuide();
      } else if (destinationPrompt) {
        event.preventDefault();
        cancelDestinationPrompt();
      } else if (passwordPromptAction) {
        event.preventDefault();
        cancelArchivePasswordPrompt();
      } else if (archiveTextPrompt) {
        event.preventDefault();
        cancelArchiveTextPrompt();
      } else if (activeAppPanel) {
        event.preventDefault();
        closeAppPanel();
      } else if (archiveContextMenu) {
        event.preventDefault();
        closeArchiveContextMenu();
      } else if (archiveInfoPanelOpen) {
        event.preventDefault();
        archiveInfoPanelOpen = false;
      } else if (extractTask && !isExtractTaskActive()) {
        event.preventDefault();
        closeExtractProgress();
      }
      return;
    }

    if (!(event.metaKey || event.ctrlKey) || event.altKey || permissionGuideOpen || passwordPromptAction || destinationPrompt) return;

    if (key === "o") {
      event.preventDefault();
      if (!busy) void pickArchive();
    } else if (key === "n") {
      event.preventDefault();
      if (!busy) void pickCompressSources();
    } else if (key === "e") {
      event.preventDefault();
      if (!busy && view === "extract" && archiveInfo) void extractCurrentArchive();
    } else if (key === "f") {
      event.preventDefault();
      if (view === "home") openRecentPanel(true);
    } else if (key === ",") {
      event.preventDefault();
      if (view === "home") openSettingsPanel();
    }
  }

  function handleGlobalContextMenu(event: MouseEvent) {
    event.preventDefault();
  }

  async function handlePackoMenuAction(action: PackoMenuAction) {
    if (permissionGuideOpen || passwordPromptAction || destinationPrompt) return;
    if (isTauriRuntime() && !(await getCurrentWindow().isFocused())) return;
    if (action === "openArchive") {
      if (!busy) await pickArchive();
    } else if (action === "newArchive") {
      if (!busy) await openBlankCompressWorkspace();
    } else if (action === "extractArchive") {
      if (!busy && view === "extract" && archiveInfo) {
        await extractCurrentArchive();
      } else if (!busy) {
        await pickArchive();
      }
    } else if (action === "showRecent") {
      if (view === "home") openRecentPanel(true);
    } else if (action === "showSettings") {
      if (view === "home") openSettingsPanel();
    }
  }

  async function openRecentFile(path: string) {
    closeAppPanel();
    await openExtractWorkspace(path);
  }

  function clearRecentFiles() {
    recentFiles = [];
  }

  function filteredRecentFiles() {
    const query = homeSearch.trim().toLowerCase();
    if (!query) return recentFiles;
    return recentFiles.filter((file) => {
      return file.name.toLowerCase().includes(query) || file.path.toLowerCase().includes(query);
    });
  }

  function searchRecentFiles() {
    const query = homeSearch.trim().toLowerCase();
    homeSearchMessage = "";
    if (!query) {
      homeSearchMessage = "输入文件名或路径后搜索。";
      return;
    }

    const count = filteredRecentFiles().length;
    if (count === 0) {
      homeSearchMessage = "最近文件中没有匹配项。";
      return;
    }

    homeSearchMessage = `找到 ${count} 个匹配项。`;
  }

  function hasArchiveSafetyWarnings() {
    const safety = archiveInfo?.safety;
    return Boolean(safety && (safety.unsafe_paths > 0 || safety.hidden_files > 0 || safety.executables > 0));
  }

  function archiveSafetyText() {
    const safety = archiveInfo?.safety;
    if (!safety) return "";
    const parts: string[] = [];
    if (safety.unsafe_paths > 0) parts.push(`${safety.unsafe_paths} 个不安全路径`);
    if (safety.hidden_files > 0) parts.push(`${safety.hidden_files} 个隐藏项目`);
    if (safety.executables > 0) parts.push(`${safety.executables} 个可执行项目`);
    return parts.join("、");
  }

  function isExtractTaskActive() {
    return Boolean(extractTask && !finishedExtractStatuses.has(extractTask.status));
  }

  function extractProgressPercent() {
    if (!extractTask || extractTask.total_bytes <= 0) return 0;
    return Math.min(100, Math.round((extractTask.completed_bytes / extractTask.total_bytes) * 100));
  }

  function currentFileProgressPercent() {
    if (!extractTask || extractTask.current_total_bytes <= 0) {
      return extractTask?.status === "completed" ? 100 : 0;
    }
    return Math.min(100, Math.round((extractTask.current_bytes / extractTask.current_total_bytes) * 100));
  }

  function archivePromiseDragPercent() {
    if (!archivePromiseDragProgress || archivePromiseDragProgress.total <= 0) return 0;
    if (archivePromiseDragProgress.status === "completed" && archivePromiseDragProgress.completed >= archivePromiseDragProgress.total) return 100;
    const percent = Math.round((archivePromiseDragProgress.completed / archivePromiseDragProgress.total) * 100);
    return Math.min(100, Math.max(archivePromiseDragProgress.status === "running" ? 12 : 0, percent));
  }

  function archivePromiseDragTitle() {
    if (!archivePromiseDragProgress) return "";
    if (archivePromiseDragProgress.status === "failed") return "拖出失败";
    if (archivePromiseDragProgress.status === "completed" && archivePromiseDragProgress.completed >= archivePromiseDragProgress.total) return "拖出完成";
    if (archivePromiseDragProgress.dragId.startsWith("pending-")) return "准备拖出";
    return "正在拖出项目";
  }

  function clearArchivePromiseDragHideTimer() {
    if (!archivePromiseDragHideTimer) return;
    window.clearTimeout(archivePromiseDragHideTimer);
    archivePromiseDragHideTimer = null;
  }

  function scheduleArchivePromiseDragHide(delay: number, dragId = archivePromiseDragProgress?.dragId) {
    clearArchivePromiseDragHideTimer();
    if (!dragId) return;
    archivePromiseDragHideTimer = window.setTimeout(() => {
      if (archivePromiseDragProgress?.dragId === dragId) {
        archivePromiseDragProgress = null;
      }
      archivePromiseDragHideTimer = null;
    }, delay);
  }

  function handleArchivePromiseDragProgress(progress: ArchivePromiseDragProgress) {
    archivePromiseDragProgress = progress;
    if (progress.status === "failed") {
      errorMessage = progress.error || progress.message;
      scheduleArchivePromiseDragHide(6000, progress.dragId);
      return;
    }
    if (progress.status === "completed" && progress.completed >= progress.total) {
      scheduleArchivePromiseDragHide(1800, progress.dragId);
      return;
    }
    clearArchivePromiseDragHideTimer();
  }

  function extractStatusLabel(status: ExtractTaskStatus) {
    if (extractTaskMode === "compress") {
      if (status === "running") return "正在压缩";
      if (status === "paused") return "压缩已暂停";
      if (status === "canceling") return "正在取消压缩";
      if (status === "completed") return "压缩完成";
      if (status === "failed") return "压缩失败";
      return "已取消压缩";
    }
    if (extractTaskMode === "edit") {
      if (status === "running") return "正在保存修改";
      if (status === "completed") return "修改已保存";
      if (status === "failed") return "保存失败";
      if (status === "canceled") return "已取消保存";
      return "正在保存修改";
    }
    if (extractTaskMode === "preview") {
      if (status === "running") return "正在准备预览";
      if (status === "paused") return "预览已暂停";
      if (status === "canceling") return "正在取消预览";
      if (status === "completed") return "预览已准备好";
      if (status === "failed") return "预览失败";
      return "已取消预览";
    }
    if (status === "running") return "正在解压";
    if (status === "paused") return "已暂停";
    if (status === "canceling") return "正在取消";
    if (status === "completed") return "解压完成";
    if (status === "failed") return "解压失败";
    return "已取消";
  }

  async function showCompletionNotification(title: string, body: string, outputPath: string) {
    if (typeof window === "undefined" || typeof Notification === "undefined") return;
    let permission = Notification.permission;
    if (permission === "default") {
      permission = await Notification.requestPermission();
    }
    if (permission !== "granted") return;

    const notification = new Notification(title, { body });
    notification.onclick = () => {
      window.focus();
      if (outputPath) {
        void revealItemInDir(outputPath).catch(() => undefined);
      }
      notification.close();
    };
  }

  function notifyCompletedTask(progress: ExtractTaskProgress) {
    if (extractTaskMode === "preview" || extractTaskMode === "edit") return;
    const isCompress = extractTaskMode === "compress";
    const title = isCompress ? "压缩完成" : "解压完成";
    const body = isCompress
      ? `${extractTaskArchiveName || "压缩包"} 已创建完成。`
      : `${extractTaskArchiveName || "压缩包"} 已解压完成。`;
    void showCompletionNotification(title, body, progress.output_path);
  }

  function stopExtractProgressPolling() {
    if (extractProgressTimer) {
      window.clearInterval(extractProgressTimer);
      extractProgressTimer = null;
    }
  }

  function stopArchiveEditProgressTimer() {
    if (archiveEditProgressTimer) {
      window.clearInterval(archiveEditProgressTimer);
      archiveEditProgressTimer = null;
    }
  }

  function archiveEditProgressPhase(percent: number, label: string) {
    if (percent < 18) return { completed: 0, currentItem: "准备编辑环境", message: `正在准备${label}。` };
    if (percent < 48) return { completed: 1, currentItem: "展开压缩包内容", message: "正在展开压缩包内容。" };
    if (percent < 62) return { completed: 2, currentItem: "应用文件树修改", message: "正在应用文件树修改。" };
    if (percent < 86) return { completed: 3, currentItem: "重新生成压缩包", message: "正在重新生成压缩包。" };
    return { completed: 4, currentItem: "写回原压缩包", message: "正在写回压缩包。" };
  }

  function startArchiveEditProgress(label: string) {
    stopExtractProgressPolling();
    stopArchiveEditProgressTimer();
    extractTaskMode = "edit";
    extractTaskArchiveName = archiveInfo?.name || "压缩包";
    extractTaskSelectionCount = 0;
    handledCompletedTaskId = "";
    const totalBytes = 100;
    extractTask = {
      task_id: `edit-${Date.now()}`,
      status: "running",
      total: 5,
      completed: 0,
      total_bytes: totalBytes,
      completed_bytes: 3,
      current_bytes: 3,
      current_total_bytes: totalBytes,
      current_item: "准备编辑环境",
      output_path: archiveInfo?.path || "",
      message: `正在准备${label}。`,
      error: null,
    };

    archiveEditProgressTimer = window.setInterval(() => {
      if (!extractTask || extractTaskMode !== "edit" || extractTask.status !== "running") {
        stopArchiveEditProgressTimer();
        return;
      }
      const current = extractTask.completed_bytes;
      const increment = current < 30 ? 7 : current < 62 ? 4 : current < 82 ? 2 : 1;
      const next = Math.min(94, current + increment);
      const phase = archiveEditProgressPhase(next, label);
      extractTask = {
        ...extractTask,
        completed: phase.completed,
        completed_bytes: next,
        current_bytes: next,
        current_item: phase.currentItem,
        message: phase.message,
      };
    }, 220);
  }

  function finishArchiveEditProgress(outputPath: string, message: string) {
    stopArchiveEditProgressTimer();
    extractTask = {
      task_id: extractTask?.task_id || `edit-${Date.now()}`,
      status: "completed",
      total: 5,
      completed: 5,
      total_bytes: 100,
      completed_bytes: 100,
      current_bytes: 100,
      current_total_bytes: 100,
      current_item: "已保存修改",
      output_path: outputPath,
      message,
      error: null,
    };
    busy = false;
  }

  function failArchiveEditProgress(message: string) {
    stopArchiveEditProgressTimer();
    extractTask = {
      task_id: extractTask?.task_id || `edit-${Date.now()}`,
      status: "failed",
      total: 5,
      completed: extractTask?.completed ?? 0,
      total_bytes: 100,
      completed_bytes: extractTask?.completed_bytes ?? 0,
      current_bytes: extractTask?.current_bytes ?? 0,
      current_total_bytes: 100,
      current_item: extractTask?.current_item || "保存修改",
      output_path: archiveInfo?.path || "",
      message,
      error: message,
    };
    busy = false;
  }

  function applyExtractProgress(progress: ExtractTaskProgress) {
    extractTask = progress;
    if (finishedExtractStatuses.has(progress.status)) {
      busy = false;
      stopExtractProgressPolling();
      const failureMessage = progress.error || progress.message;
      if (progress.status === "failed" && extractTaskMode !== "compress" && isArchivePasswordError(failureMessage)) {
        const action = activePasswordRetryAction;
        activePasswordRetryAction = null;
        if (action) {
          requestArchivePassword(action, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
          return;
        }
      }
      if (progress.status === "completed") {
        statusMessage = "";
        errorMessage = "";
        activePasswordRetryAction = null;
        if (progress.task_id !== handledCompletedTaskId) {
          handledCompletedTaskId = progress.task_id;
          if (extractTaskMode === "preview") {
            if (previewTaskEntryPath) {
              previewCache = new Map(previewCache).set(previewTaskEntryPath, progress.output_path);
            }
            void openPath(progress.output_path).catch((error) => {
              errorMessage = String(error);
            });
          } else {
            notifyCompletedTask(progress);
            if (extractTaskMode === "compress") {
              addRecentArchivePath(progress.output_path, {
                name: progress.output_path.split(/[\\/]/).pop() || progress.output_path,
                time: currentRecentTimeLabel(),
              });
            }
          }
        }
      } else if (progress.error) {
        activePasswordRetryAction = null;
        errorMessage = progress.error;
      } else {
        activePasswordRetryAction = null;
      }
    } else {
      busy = true;
    }
  }

  async function refreshExtractProgress(taskId: string) {
    const command = extractTaskMode === "compress" ? "get_compress_task" : "get_extract_task";
    const progress = await invoke<ExtractTaskProgress>(command, { taskId });
    applyExtractProgress(progress);
  }

  function startExtractProgressPolling(taskId: string) {
    stopExtractProgressPolling();
    extractProgressTimer = window.setInterval(() => {
      void refreshExtractProgress(taskId).catch((error) => {
        stopExtractProgressPolling();
        busy = false;
        errorMessage = String(error);
      });
    }, 350);
  }

  function isArchivePathSelected(path: string) {
    return selectedArchivePaths.includes(path);
  }

  function flattenArchiveTree(nodes = archiveTree): ArchiveRow[] {
    const rows: ArchiveRow[] = [];
    for (const node of nodes) {
      rows.push(node);
      if (node.kind === "folder" && expandedArchiveFolders.includes(node.path)) {
        rows.push(...flattenArchiveTree(node.children));
      }
    }
    return rows;
  }

  function allArchiveRows(nodes = archiveTree): ArchiveRow[] {
    return nodes.flatMap((node) => [node, ...allArchiveRows(node.children)]);
  }

  function archiveFilterActive() {
    return archiveSearchQuery.trim().length > 0 || archiveKindFilter !== "all";
  }

  function archiveRowMatchesFilter(row: ArchiveRow) {
    const query = archiveSearchQuery.trim().toLowerCase();
    const matchesQuery = !query || `${row.name} ${row.path} ${row.type}`.toLowerCase().includes(query);
    if (!matchesQuery) return false;

    if (archiveKindFilter === "files") return row.kind !== "folder";
    if (archiveKindFilter === "folders") return row.kind === "folder";
    if (archiveKindFilter === "encrypted") return row.isEncrypted;
    if (archiveKindFilter === "hidden") return row.isHidden;
    if (archiveKindFilter === "executable") return row.isExecutable;
    return true;
  }

  function archiveAncestorPaths(path: string) {
    const ancestors: string[] = [];
    let parent = archiveParentPath(path);
    while (parent) {
      ancestors.push(parent);
      parent = archiveParentPath(parent);
    }
    return ancestors;
  }

  function flattenFilteredArchiveTree(nodes: ArchiveTreeNode[], includedPaths: Set<string>): ArchiveRow[] {
    const rows: ArchiveRow[] = [];
    for (const node of nodes) {
      if (includedPaths.has(node.path)) {
        rows.push(node);
      }
      rows.push(...flattenFilteredArchiveTree(node.children, includedPaths));
    }
    return rows;
  }

  function visibleArchiveRows() {
    if (!archiveFilterActive()) return flattenArchiveTree();
    const includedPaths = new Set<string>();
    for (const row of allArchiveRows()) {
      if (!archiveRowMatchesFilter(row)) continue;
      includedPaths.add(row.path);
      for (const parent of archiveAncestorPaths(row.path)) {
        includedPaths.add(parent);
      }
    }
    return flattenFilteredArchiveTree(archiveTree, includedPaths);
  }

  function isArchivePathSelectable(path: string) {
    return archiveSelectableFileRowsForPath(path).length > 0;
  }

  function archiveSelectableRows() {
    if (!archiveFilterActive()) return allArchiveFileRows();

    const rows = new Map<string, ArchiveRow>();
    for (const row of allArchiveRows()) {
      if (!archiveRowMatchesFilter(row)) continue;
      for (const fileRow of archiveSelectableFileRowsForPath(row.path)) {
        rows.set(fileRow.path, fileRow);
      }
    }
    return Array.from(rows.values());
  }

  function allArchiveFileRows() {
    return allArchiveRows().filter((row) => row.kind !== "folder");
  }

  function archiveSelectableFileRowsForPath(path: string) {
    const row = allArchiveRows().find((item) => item.path === path);
    if (!row) return [];
    if (row.kind !== "folder") return [row];
    const prefix = `${row.path.replace(/\/+$/, "")}/`;
    return allArchiveFileRows().filter((item) => item.path.startsWith(prefix));
  }

  function archiveRowSelectedCount(row: ArchiveRow) {
    const selected = new Set(selectedArchivePaths);
    const fileRows = archiveSelectableFileRowsForPath(row.path);
    return fileRows.filter((item) => selected.has(item.path)).length;
  }

  function isArchiveRowSelected(row: ArchiveRow) {
    const fileRows = archiveSelectableFileRowsForPath(row.path);
    return fileRows.length > 0 && archiveRowSelectedCount(row) === fileRows.length;
  }

  function isArchiveRowPartiallySelected(row: ArchiveRow) {
    const fileRows = archiveSelectableFileRowsForPath(row.path);
    const selectedCount = archiveRowSelectedCount(row);
    return fileRows.length > 0 && selectedCount > 0 && selectedCount < fileRows.length;
  }

  function isArchiveRowHighlighted(row: ArchiveRow) {
    return row.kind === "folder" ? isArchiveRowSelected(row) : isArchivePathSelected(row.path);
  }

  function selectedArchiveEntries() {
    const selected = new Set(selectedArchivePaths);
    return allArchiveFileRows().filter((row) => selected.has(row.path));
  }

  function toggleArchivePath(path: string, force?: boolean) {
    if (!path || !isArchivePathSelectable(path)) return;
    const fileRows = archiveSelectableFileRowsForPath(path);
    if (fileRows.length === 0) return;
    const selected = new Set(selectedArchivePaths);
    const allSelected = fileRows.every((row) => selected.has(row.path));
    const shouldSelect = force ?? !allSelected;
    for (const row of fileRows) {
      if (shouldSelect) {
        selected.add(row.path);
      } else {
        selected.delete(row.path);
      }
    }
    selectedArchivePaths = Array.from(selected);
  }

  function toggleAllArchiveRows() {
    const paths = archiveSelectableRows().map((row) => row.path);
    if (paths.length === 0) return;
    const selected = new Set(selectedArchivePaths);
    const allVisibleSelected = paths.every((path) => selected.has(path));
    selectedArchivePaths = allVisibleSelected
      ? selectedArchivePaths.filter((path) => !paths.includes(path))
      : Array.from(new Set([...selectedArchivePaths, ...paths]));
  }

  function archiveTableColumnsStyle() {
    return `--archive-table-columns: 26px minmax(${archiveColumnMin.name}px, 1fr) minmax(${archiveColumnMin.type}px, ${archiveColumnWidths.type}px) minmax(${archiveColumnMin.size}px, ${archiveColumnWidths.size}px) minmax(${archiveColumnMin.modified}px, ${archiveColumnWidths.modified}px);`;
  }

  function archiveColumnLabel(column: ArchiveColumnKey) {
    return {
      name: "名称",
      type: "类型",
      size: "大小",
      modified: "修改时间",
    }[column];
  }

  function archiveSortTitle(column: ArchiveColumnKey) {
    if (archiveSort.column !== column) return `按${archiveColumnLabel(column)}排序`;
    return archiveSort.direction === "asc"
      ? `${archiveColumnLabel(column)}升序，点击切换为降序`
      : `${archiveColumnLabel(column)}降序，点击切换为升序`;
  }

  function setArchiveSort(column: ArchiveColumnKey) {
    const direction: ArchiveSortDirection = archiveSort.column === column && archiveSort.direction === "asc" ? "desc" : "asc";
    archiveSort = { column, direction };
    archiveTree = sortArchiveTree(archiveTree);
  }

  function clampArchiveColumnWidth(column: ArchiveColumnKey, width: number) {
    return Math.min(archiveColumnMax[column], Math.max(archiveColumnMin[column], Math.round(width)));
  }

  function resizePairRightColumn(column: ArchiveColumnKey): ArchiveColumnKey | null {
    if (column === "name") return "type";
    if (column === "type") return "size";
    if (column === "size") return "modified";
    return null;
  }

  function startArchiveColumnResize(column: ArchiveColumnKey, event: MouseEvent) {
    if (event.button !== 0) return;
    const right = resizePairRightColumn(column);
    if (!right) return;
    const leftCell = (event.currentTarget as HTMLElement).closest<HTMLElement>(".archive-head-cell");
    const rightCell = leftCell?.nextElementSibling as HTMLElement | null;
    const startLeftWidth = leftCell?.getBoundingClientRect().width || archiveColumnWidths[column];
    const startRightWidth = rightCell?.getBoundingClientRect().width || archiveColumnWidths[right];
    event.preventDefault();
    event.stopPropagation();
    archiveColumnResize = {
      left: column,
      right,
      startX: event.clientX,
      startLeftWidth,
      startRightWidth,
    };
  }

  function moveArchiveColumnResize(event: MouseEvent) {
    if (!archiveColumnResize) return false;
    event.preventDefault();
    const rawDelta = event.clientX - archiveColumnResize.startX;
    const leftMaxDelta = archiveColumnResize.left === "name"
      ? Number.POSITIVE_INFINITY
      : archiveColumnMax[archiveColumnResize.left] - archiveColumnResize.startLeftWidth;
    const minDelta = Math.max(
      archiveColumnMin[archiveColumnResize.left] - archiveColumnResize.startLeftWidth,
      archiveColumnResize.startRightWidth - archiveColumnMax[archiveColumnResize.right]
    );
    const maxDelta = Math.min(
      leftMaxDelta,
      archiveColumnResize.startRightWidth - archiveColumnMin[archiveColumnResize.right]
    );
    const delta = Math.min(maxDelta, Math.max(minDelta, rawDelta));
    const nextLeftWidth = clampArchiveColumnWidth(archiveColumnResize.left, archiveColumnResize.startLeftWidth + delta);
    const nextRightWidth = clampArchiveColumnWidth(archiveColumnResize.right, archiveColumnResize.startRightWidth - delta);
    archiveColumnWidths = {
      ...archiveColumnWidths,
      [archiveColumnResize.left]: nextLeftWidth,
      [archiveColumnResize.right]: nextRightWidth,
    };
    return true;
  }

  function stopArchiveColumnResize() {
    archiveColumnResize = null;
  }

  function startArchiveRowSelection(path: string, event: MouseEvent) {
    if (event.button !== 0) return;
    if (!isArchivePathSelectable(path)) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("button, input, [data-row-action]")) return;
    const row = allArchiveRows().find((item) => item.path === path);
    if (row?.kind === "folder") return;

    archiveDragMode = isArchivePathSelected(path) ? "deselect" : "select";
    archiveDragSelecting = true;
    toggleArchivePath(path, archiveDragMode === "select");
  }

  function moveArchiveRowSelection(path: string) {
    if (!archiveDragSelecting) return;
    toggleArchivePath(path, archiveDragMode === "select");
  }

  function stopArchiveRowSelection() {
    archiveDragSelecting = false;
  }

  function openArchiveContextMenu(row: ArchiveRow | null, event: MouseEvent) {
    if (!archiveInfo) return;
    event.preventDefault();
    event.stopPropagation();
    contextOpenWithApps = [];
    contextOpenWithRowPath = "";
    contextOpenWithLoading = false;
    const menuWidth = 220;
    const menuHeight = row ? 360 : 126;
    archiveContextMenu = {
      x: Math.min(event.clientX, window.innerWidth - menuWidth - 10),
      y: Math.min(event.clientY, window.innerHeight - menuHeight - 10),
      row,
    };
  }

  function closeArchiveContextMenu() {
    archiveContextMenu = null;
    contextOpenWithApps = [];
    contextOpenWithRowPath = "";
    contextOpenWithLoading = false;
  }

  function archiveRowsByPaths(paths: string[]) {
    const selected = new Set(paths);
    return allArchiveRows().filter((row) => selected.has(row.path));
  }

  function archiveJoinPath(parent: string, name: string) {
    const cleanParent = parent.replace(/\/+$/, "");
    const cleanName = name.replace(/^\/+/, "");
    return cleanParent ? `${cleanParent}/${cleanName}` : cleanName;
  }

  function normalizeArchiveEntryPath(path: string) {
    return path.replace(/\\/g, "/").replace(/^\/+/, "").replace(/\/+$/, "");
  }

  function mapArchivePathAfterEdit(path: string, draft: ArchiveEditDraft) {
    let nextPath = normalizeArchiveEntryPath(path);
    if (!nextPath) return "";

    for (const deletedPath of draft.deleteEntries ?? []) {
      const cleanDeletedPath = normalizeArchiveEntryPath(deletedPath);
      if (!cleanDeletedPath) continue;
      if (nextPath === cleanDeletedPath || nextPath.startsWith(`${cleanDeletedPath}/`)) {
        return null;
      }
    }

    for (const entry of draft.renameEntries ?? []) {
      const from = normalizeArchiveEntryPath(entry.from);
      const to = normalizeArchiveEntryPath(entry.to);
      if (!from || !to) continue;
      if (nextPath === from) {
        nextPath = to;
      } else if (nextPath.startsWith(`${from}/`)) {
        nextPath = archiveJoinPath(to, nextPath.slice(from.length + 1));
      }
    }

    return nextPath;
  }

  function restoredArchiveExpandedFolders(previousExpandedFolders: string[], draft: ArchiveEditDraft) {
    const folderPaths = new Set(allArchiveRows().filter((row) => row.kind === "folder").map((row) => row.path));
    const restored = new Set(defaultExpandedFolders(archiveTree));

    for (const path of previousExpandedFolders) {
      const mappedPath = mapArchivePathAfterEdit(path, draft);
      if (mappedPath && folderPaths.has(mappedPath)) {
        restored.add(mappedPath);
      }
    }

    for (const entry of draft.renameEntries ?? []) {
      const targetParent = archiveParentPath(normalizeArchiveEntryPath(entry.to));
      if (targetParent && folderPaths.has(targetParent)) {
        restored.add(targetParent);
      }
    }

    for (const dir of draft.createDirs ?? []) {
      const folderPath = normalizeArchiveEntryPath(dir);
      const parentPath = archiveParentPath(folderPath);
      if (parentPath && folderPaths.has(parentPath)) {
        restored.add(parentPath);
      }
      if (folderPaths.has(folderPath)) {
        restored.add(folderPath);
      }
    }

    return Array.from(restored);
  }

  function archiveAncestorPathAtDepth(path: string, depth: number) {
    if (depth < 0) return "";
    return normalizeArchiveEntryPath(path)
      .split("/")
      .slice(0, depth + 1)
      .join("/");
  }

  function archiveDropTargetPathFromPoint(clientX: number, clientY: number) {
    const element = document.elementFromPoint(clientX, clientY);
    const rowElement = element?.closest<HTMLElement>("[data-archive-row-path]");
    if (rowElement) {
      const rowPath = rowElement.dataset.archiveRowPath || "";
      const row = allArchiveRows().find((item) => item.path === rowPath);
      const fileCell = rowElement.querySelector<HTMLElement>(".archive-file-cell");
      if (row && fileCell) {
        const indent = 14;
        const depthFromX = Math.floor((clientX - fileCell.getBoundingClientRect().left + indent / 2) / indent);
        if (row.kind === "folder") {
          return archiveAncestorPathAtDepth(row.path, Math.min(row.depth, depthFromX));
        }
        return archiveAncestorPathAtDepth(row.path, Math.min(row.depth - 1, depthFromX - 1));
      }
    }
    return element?.closest(".archive-list-panel") ? "" : null;
  }

  function archiveClientPointFromTauriPosition(position: { x: number; y: number }) {
    const scale = window.devicePixelRatio || 1;
    return {
      x: position.x / scale,
      y: position.y / scale,
    };
  }

  function archiveDragSourceRows(row: ArchiveRow) {
    if (row.kind === "folder") return [row];
    if (selectedArchivePaths.includes(row.path)) {
      const rows = archiveRowsByPaths(selectedArchivePaths);
      if (rows.length > 0) return rows;
    }
    return [row];
  }

  function isArchiveMoveTargetValid(rows: ArchiveRow[], targetPath: string | null) {
    if (targetPath === null || rows.length === 0) return false;
    if (targetPath) {
      const target = allArchiveRows().find((row) => row.path === targetPath);
      if (!target || target.kind !== "folder") return false;
    }

    let hasMove = false;
    for (const row of rows) {
      if (row.path === targetPath) return false;
      if (row.kind === "folder" && targetPath.startsWith(`${row.path}/`)) return false;
      if (archiveParentPath(row.path) !== targetPath) hasMove = true;
    }
    return hasMove;
  }

  function setArchiveInternalDropTargetFromPoint(clientX: number, clientY: number) {
    const candidate = archiveFileDragCandidate;
    if (!candidate?.started) {
      archiveInternalDropTargetPath = null;
      return;
    }
    const targetPath = archiveDropTargetPathFromPoint(clientX, clientY);
    archiveInternalDropTargetPath = isArchiveMoveTargetValid(candidate.rows, targetPath) ? targetPath : null;
  }

  function setArchiveExternalDropTargetFromPosition(position: { x: number; y: number } | null) {
    if (!position || view !== "extract" || !archiveInfo) {
      archiveExternalDropTargetPath = null;
      return;
    }
    const point = archiveClientPointFromTauriPosition(position);
    const targetPath = archiveDropTargetPathFromPoint(point.x, point.y);
    if (targetPath === "") {
      archiveExternalDropTargetPath = "";
      return;
    }
    const target = targetPath ? allArchiveRows().find((row) => row.path === targetPath) : null;
    archiveExternalDropTargetPath = target?.kind === "folder" ? target.path : null;
  }

  async function moveArchiveRowsToFolder(rows: ArchiveRow[], targetDir: string) {
    if (!archiveInfo || busy || rows.length === 0) return;
    clearStatus();
    closeArchiveContextMenu();

    const existingPaths = new Set(allArchiveRows().map((row) => row.path));
    const nextPaths = new Set<string>();
    const movingSourcePaths = new Set(rows.filter((row) => archiveParentPath(row.path) !== targetDir).map((row) => row.path));
    const renameEntries: ArchiveRenameEntry[] = [];

    for (const row of rows) {
      if (row.path === targetDir || (row.kind === "folder" && targetDir.startsWith(`${row.path}/`))) {
        errorMessage = "不能移动到自身或子文件夹中。";
        return;
      }
      if (archiveParentPath(row.path) === targetDir) continue;

      const nextPath = archiveJoinPath(targetDir, row.name);
      if (nextPaths.has(nextPath) || (existingPaths.has(nextPath) && !movingSourcePaths.has(nextPath))) {
        errorMessage = "目标文件夹中已有同名项目。";
        return;
      }
      nextPaths.add(nextPath);
      renameEntries.push({ from: row.path, to: nextPath });
    }

    if (renameEntries.length === 0) {
      statusMessage = "项目已在目标文件夹中。";
      return;
    }

    await runArchiveEdit({ renameEntries }, "移动");
  }

  function contextArchiveRows() {
    const row = archiveContextMenu?.row;
    if (!row) return selectedArchiveEntries();
    if (row.kind !== "folder" && selectedArchivePaths.includes(row.path)) {
      const rows = archiveRowsByPaths(selectedArchivePaths);
      if (rows.length > 1) return rows;
    }
    return [row];
  }

  function contextDeleteLabel() {
    const rows = contextArchiveRows();
    if (rows.length > 1) return `删除所选 ${rows.length} 项`;
    return "删除";
  }

  function archiveContextTargetDir() {
    const contextRow = archiveContextMenu?.row;
    if (!contextRow) return "";
    return contextRow.kind === "folder" ? contextRow.path : archiveParentPath(contextRow.path);
  }

  function uniqueArchiveFolderName(parentDir: string) {
    const existingPaths = new Set(allArchiveRows().map((row) => row.path));
    const baseName = "新建文件夹";
    let candidate = baseName;
    let index = 2;
    while (existingPaths.has(archiveJoinPath(parentDir, candidate))) {
      candidate = `${baseName} ${index}`;
      index += 1;
    }
    return candidate;
  }

  async function previewContextArchiveRow() {
    const row = archiveContextMenu?.row;
    if (!row || row.kind === "folder") return;
    closeArchiveContextMenu();
    await previewArchiveRow(row);
  }

  async function ensureArchivePreviewPath(row: ArchiveRow, retryAction: ArchivePasswordAction) {
    if (!archiveInfo) {
      errorMessage = "请先选择压缩包。";
      return null;
    }
    if (row.kind === "folder") return null;

    const cached = previewCache.get(row.path);
    if (cached) return cached;

    try {
      const result = await invoke<OperationResult>("preview_archive_entry", {
        path: archiveInfo.path,
        entryPath: row.path,
        password: extractPasswordParam(),
      });
      previewCache = new Map(previewCache).set(row.path, result.output_path);
      return result.output_path;
    } catch (error) {
      const message = String(error);
      if (isArchivePasswordError(message)) {
        requestArchivePassword(retryAction, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return null;
      }
      errorMessage = message;
      return null;
    }
  }

  async function openArchiveRowWithApplication(row: ArchiveRow, applicationPath: string | null) {
    clearStatus();
    const outputPath = await ensureArchivePreviewPath(row, { kind: "openWith", row, applicationPath });
    if (!outputPath) return;
    closeArchiveContextMenu();

    try {
      await invoke("open_file_with_application", {
        path: outputPath,
        applicationPath,
      });
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function loadContextOpenWithApps(row: ArchiveRow) {
    if (row.kind === "folder") return;
    clearStatus();
    contextOpenWithRowPath = row.path;
    contextOpenWithApps = [];
    contextOpenWithLoading = true;

    const outputPath = await ensureArchivePreviewPath(row, { kind: "openWithSuggestions", row });
    if (!outputPath) {
      contextOpenWithLoading = false;
      return;
    }

    try {
      contextOpenWithApps = await invoke<OpenWithApplication[]>("suggest_open_with_apps", { path: outputPath });
    } catch (error) {
      errorMessage = String(error);
      contextOpenWithApps = [];
    } finally {
      contextOpenWithLoading = false;
    }
  }

  async function toggleContextOpenWithApps() {
    const row = archiveContextMenu?.row;
    if (!row || row.kind === "folder") return;
    if (contextOpenWithRowPath === row.path && !contextOpenWithLoading) {
      contextOpenWithRowPath = "";
      contextOpenWithApps = [];
      return;
    }
    await loadContextOpenWithApps(row);
  }

  async function pickContextOpenWithApplication() {
    const row = archiveContextMenu?.row;
    if (!row || row.kind === "folder") return;

    try {
      const application = await invoke<OpenWithApplication | null>("pick_open_with_application");
      if (!application) return;
      await openArchiveRowWithApplication(row, application.path);
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function openContextRowWithApplication(applicationPath: string | null) {
    const row = archiveContextMenu?.row;
    if (!row || row.kind === "folder") return;
    await openArchiveRowWithApplication(row, applicationPath);
  }

  async function renameContextArchiveRow() {
    const row = archiveContextMenu?.row;
    if (!row) return;
    await renameArchiveRow(row);
  }

  async function replaceContextArchiveRow() {
    const row = archiveContextMenu?.row;
    if (!row) return;
    await replaceArchiveRow(row);
  }

  function archiveTextPromptTitle(prompt: ArchiveTextPrompt) {
    return prompt.kind === "rename" ? "重命名" : "新建文件夹";
  }

  function archiveTextPromptDescription(prompt: ArchiveTextPrompt) {
    if (prompt.kind === "rename") {
      const parentPath = archiveParentPath(prompt.row.path);
      return parentPath ? `位于 ${parentPath}` : "位于压缩包根目录";
    }
    return prompt.targetDir ? `创建到 ${prompt.targetDir}` : "创建到压缩包根目录";
  }

  function archiveTextPromptLabel(prompt: ArchiveTextPrompt) {
    return prompt.kind === "rename" ? "名称" : "文件夹名称";
  }

  function openArchiveTextPrompt(prompt: ArchiveTextPrompt, value: string) {
    closeArchiveContextMenu();
    clearStatus();
    archiveTextPrompt = prompt;
    archiveTextPromptValue = value;
    archiveTextPromptMessage = "";
    window.setTimeout(() => {
      archiveTextPromptInput?.focus();
      archiveTextPromptInput?.select();
    }, 0);
  }

  function cancelArchiveTextPrompt() {
    archiveTextPrompt = null;
    archiveTextPromptValue = "";
    archiveTextPromptMessage = "";
  }

  function archiveTextPathIsSafe(path: string) {
    return path.split("/").every((part) => part && part !== "." && part !== "..");
  }

  async function submitArchiveTextPrompt() {
    const prompt = archiveTextPrompt;
    if (!prompt || busy) return;
    const value = normalizeArchiveEntryPath(archiveTextPromptValue);
    if (!value) {
      archiveTextPromptMessage = prompt.kind === "rename" ? "请输入新的名称。" : "请输入文件夹名称。";
      return;
    }
    if (!archiveTextPathIsSafe(value)) {
      archiveTextPromptMessage = "路径不安全。";
      return;
    }

    if (prompt.kind === "rename") {
      const row = prompt.row;
      if (value.includes("/")) {
        archiveTextPromptMessage = "这里只能修改名称，移动位置请直接拖拽。";
        return;
      }
      if (value === row.name) {
        cancelArchiveTextPrompt();
        return;
      }
      const nextPath = archiveJoinPath(archiveParentPath(row.path), value);
      if (allArchiveRows().some((item) => item.path === nextPath && item.path !== row.path)) {
        archiveTextPromptMessage = "目标路径已存在。";
        return;
      }
      archiveTextPromptMessage = "正在保存...";
      const saved = await runArchiveEdit({ renameEntries: [{ from: row.path, to: nextPath }] }, "重命名");
      if (!saved) {
        archiveTextPromptMessage = errorMessage || "重命名失败。";
        return;
      }
      if (!allArchiveRows().some((item) => item.path === nextPath)) {
        archiveTextPromptMessage = "重命名已提交，但列表没有刷新到新名称。";
        return;
      }
      cancelArchiveTextPrompt();
      return;
    }

    const folderPath = archiveJoinPath(prompt.targetDir, value);
    if (allArchiveRows().some((row) => row.path === folderPath)) {
      archiveTextPromptMessage = "目标文件夹已存在。";
      return;
    }
    archiveTextPromptMessage = "正在保存...";
    const saved = await runArchiveEdit({ createDirs: [folderPath] }, "新建文件夹");
    if (!saved) {
      archiveTextPromptMessage = errorMessage || "新建文件夹失败。";
      return;
    }
    if (!allArchiveRows().some((row) => row.path === folderPath)) {
      archiveTextPromptMessage = "新建文件夹已提交，但列表没有刷新到新文件夹。";
      return;
    }
    cancelArchiveTextPrompt();
  }

  async function createArchiveFolder() {
    if (!archiveInfo || busy) return;
    const targetDir = archiveContextTargetDir();
    openArchiveTextPrompt({ kind: "createFolder", targetDir }, uniqueArchiveFolderName(targetDir));
  }

  async function renameArchiveRow(row: ArchiveRow) {
    if (!archiveInfo || busy) return;
    openArchiveTextPrompt({ kind: "rename", row }, row.name);
  }

  async function deleteArchiveRows(rows: ArchiveRow[]) {
    if (!archiveInfo || busy || rows.length === 0) return;
    closeArchiveContextMenu();
    const label = rows.length === 1 ? `删除 ${rows[0].name}？` : `删除所选 ${rows.length} 个项目？`;
    if (!window.confirm(label)) return;
    await runArchiveEdit({ deleteEntries: rows.map((row) => row.path) }, "删除");
  }

  async function deleteContextArchiveRows() {
    await deleteArchiveRows(contextArchiveRows());
  }

  async function replaceArchiveRow(row: ArchiveRow) {
    if (!archiveInfo || busy || row.kind === "folder") return;
    closeArchiveContextMenu();
    const selected = await open({ multiple: false, fileAccessMode: "scoped" });
    if (typeof selected !== "string") return;
    await runArchiveEdit({ replaceEntries: [{ entryPath: row.path, sourcePath: selected }] }, "替换");
  }

  function parseArchiveModifiedValue(label: string) {
    const match = label.match(/^(\d{4})年(\d{1,2})月(\d{1,2})日\s+(\d{1,2}):(\d{2})$/);
    if (!match) return 0;
    const [, year, month, day, hour, minute] = match;
    const value = new Date(Number(year), Number(month) - 1, Number(day), Number(hour), Number(minute)).getTime();
    return Number.isFinite(value) ? value : 0;
  }

  function compareArchiveNodes(a: ArchiveTreeNode, b: ArchiveTreeNode) {
    if (a.kind === "folder" && b.kind !== "folder") return -1;
    if (a.kind !== "folder" && b.kind === "folder") return 1;

    let result = 0;
    if (archiveSort.column === "name") {
      result = a.name.localeCompare(b.name, "zh-Hans-CN", { numeric: true, sensitivity: "base" });
    } else if (archiveSort.column === "type") {
      result = a.type.localeCompare(b.type, "zh-Hans-CN", { numeric: true, sensitivity: "base" });
    } else if (archiveSort.column === "size") {
      result = a.sizeValue - b.sizeValue;
    } else {
      result = a.modifiedValue - b.modifiedValue;
    }

    if (result === 0) {
      result = a.name.localeCompare(b.name, "zh-Hans-CN", { numeric: true, sensitivity: "base" });
    }
    return archiveSort.direction === "asc" ? result : -result;
  }

  function sortArchiveTree(nodes: ArchiveTreeNode[]): ArchiveTreeNode[] {
    return nodes
      .map((node) => ({
        ...node,
        children: sortArchiveTree(node.children),
      }))
      .sort(compareArchiveNodes);
  }

  function archiveRowFromEntry(entry: ArchiveEntry, depth: number): ArchiveRow {
    return {
      name: entry.name,
      type: entry.type_label,
      size: entry.size_label,
      sizeValue: entry.size,
      modified: entry.modified_label,
      modifiedValue: parseArchiveModifiedValue(entry.modified_label),
      kind: entry.kind,
      path: entry.path,
      depth,
      crc: entry.crc,
      method: entry.method,
      isEncrypted: entry.is_encrypted,
      isHidden: entry.is_hidden,
      isExecutable: entry.is_executable,
      isUnsafePath: entry.is_unsafe_path,
    };
  }

  function folderEntryFromPath(path: string, depth: number): ArchiveEntry {
    const cleanPath = path.replace(/\/+$/, "");
    const name = cleanPath.split("/").pop() || cleanPath;
    return {
      name,
      path: cleanPath,
      kind: "folder",
      type_label: "文件夹",
      size: 0,
      size_label: "-",
      modified_label: "-",
      crc: null,
      method: null,
      is_encrypted: false,
      is_hidden: archiveEntryIsHidden(cleanPath),
      is_executable: false,
      is_unsafe_path: false,
    };
  }

  function buildArchiveTree(entries: ArchiveEntry[]) {
    const roots: ArchiveTreeNode[] = [];
    const folderMap = new Map<string, ArchiveTreeNode>();

    function ensureFolder(path: string, depth: number) {
      const cleanPath = path.replace(/\/+$/, "");
      const existing = folderMap.get(cleanPath);
      if (existing) return existing;

      const parentPath = archiveParentPath(cleanPath);
      const node: ArchiveTreeNode = {
        ...archiveRowFromEntry(folderEntryFromPath(cleanPath, depth), depth),
        children: [],
      };
      folderMap.set(cleanPath, node);

      if (parentPath) {
        ensureFolder(parentPath, depth - 1).children.push(node);
      } else {
        roots.push(node);
      }
      return node;
    }

    for (const entry of entries) {
      const cleanPath = entry.path.replace(/\/+$/, "");
      if (!cleanPath) continue;
      const parentPath = archiveParentPath(cleanPath);
      const depth = cleanPath.split("/").length - 1;
      const node: ArchiveTreeNode = {
        ...archiveRowFromEntry({ ...entry, path: cleanPath }, depth),
        children: [],
      };

      if (entry.kind === "folder") {
        const folderNode = ensureFolder(cleanPath, depth);
        Object.assign(folderNode, node, { children: folderNode.children });
        continue;
      }

      if (parentPath) {
        ensureFolder(parentPath, depth - 1).children.push(node);
      } else {
        roots.push(node);
      }
    }

    function sortNodes(nodes: ArchiveTreeNode[]) {
      const sorted = sortArchiveTree(nodes);
      nodes.splice(0, nodes.length, ...sorted);
    }

    sortNodes(roots);
    return roots;
  }

  function defaultExpandedFolders(nodes: ArchiveTreeNode[]) {
    const expanded: string[] = [];
    if (nodes.length === 1 && nodes[0].kind === "folder") {
      expanded.push(nodes[0].path);
    }
    return expanded;
  }

  function isFolderExpanded(path: string) {
    return expandedArchiveFolders.includes(path);
  }

  function isArchiveFolderExpandedForDisplay(path: string) {
    return archiveFilterActive() || isFolderExpanded(path);
  }

  function toggleArchiveFolder(path: string) {
    expandedArchiveFolders = isFolderExpanded(path)
      ? expandedArchiveFolders.filter((item) => item !== path)
      : [...expandedArchiveFolders, path];
  }

  function normalizeLocalPath(path: string) {
    return path.replace(/\\/g, "/").replace(/\/+$/, "");
  }

  function localParentPath(path: string) {
    const normalized = normalizeLocalPath(path);
    const parts = normalized.split("/");
    if (parts.length <= 1) return "";
    if (normalized.startsWith("/") && parts.length === 2) return "/";
    return parts.slice(0, -1).join("/") || (normalized.startsWith("/") ? "/" : "");
  }

  function pathContains(parent: string, child: string) {
    const normalizedParent = normalizeLocalPath(parent);
    const normalizedChild = normalizeLocalPath(child);
    return normalizedChild === normalizedParent || normalizedChild.startsWith(`${normalizedParent}/`);
  }

  function compressSourceFromInfo(item: FileInfo): CompressSource {
    return {
      path: item.path,
      name: item.name,
      size: item.size_label,
      sizeValue: item.size,
      kind: item.kind,
    };
  }

  function buildCompressTree(items: FileInfo[]) {
    const nodeMap = new Map<string, CompressTreeNode>();
    const roots: CompressTreeNode[] = [];

    for (const item of items) {
      const path = normalizeLocalPath(item.path);
      nodeMap.set(path, {
        ...compressSourceFromInfo({ ...item, path }),
        depth: 0,
        children: [],
      });
    }

    for (const node of nodeMap.values()) {
      const parent = localParentPath(node.path);
      const parentNode = parent ? nodeMap.get(parent) : null;
      if (parentNode && parentNode !== node) {
        parentNode.children.push(node);
      } else {
        roots.push(node);
      }
    }

    function sortAndDepth(nodes: CompressTreeNode[], depth: number) {
      nodes.sort((a, b) => {
        if (a.kind === "folder" && b.kind !== "folder") return -1;
        if (a.kind !== "folder" && b.kind === "folder") return 1;
        return a.name.localeCompare(b.name, "zh-Hans-CN");
      });
      for (const node of nodes) {
        node.depth = depth;
        sortAndDepth(node.children, depth + 1);
      }
    }

    sortAndDepth(roots, 0);
    return roots;
  }

  function flattenCompressTree(nodes = compressTree): CompressTreeNode[] {
    const rows: CompressTreeNode[] = [];
    for (const node of nodes) {
      rows.push(node);
      if (node.kind === "folder" && expandedCompressFolders.includes(node.path)) {
        rows.push(...flattenCompressTree(node.children));
      }
    }
    return rows;
  }

  function allCompressRows(nodes = compressTree): CompressTreeNode[] {
    return nodes.flatMap((node) => [node, ...allCompressRows(node.children)]);
  }

  function isCompressFolderExpanded(path: string) {
    return expandedCompressFolders.includes(path);
  }

  function toggleCompressFolder(path: string) {
    expandedCompressFolders = isCompressFolderExpanded(path)
      ? expandedCompressFolders.filter((item) => item !== path)
      : [...expandedCompressFolders, path];
  }

  function isCompressPathSelected(path: string) {
    return selectedCompressPaths.includes(path);
  }

  function toggleCompressPath(path: string) {
    selectedCompressPaths = isCompressPathSelected(path)
      ? selectedCompressPaths.filter((item) => item !== path)
      : [...selectedCompressPaths, path];
  }

  function setCompressPathSelected(path: string, selected: boolean) {
    selectedCompressPaths = selected
      ? Array.from(new Set([...selectedCompressPaths, path]))
      : selectedCompressPaths.filter((item) => item !== path);
  }

  function toggleAllCompressRows() {
    const paths = allCompressRows().map((row) => row.path);
    selectedCompressPaths = selectedCompressPaths.length === paths.length ? [] : paths;
  }

  function startCompressRowSelection(path: string, event: MouseEvent) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("button, input, [data-row-action]")) return;

    compressDragMode = isCompressPathSelected(path) ? "deselect" : "select";
    compressDragSelecting = true;
    setCompressPathSelected(path, compressDragMode === "select");
  }

  function moveCompressRowSelection(path: string) {
    if (!compressDragSelecting) return;
    setCompressPathSelected(path, compressDragMode === "select");
  }

  function stopCompressRowSelection() {
    compressDragSelecting = false;
  }

  function defaultExpandedCompressFolders(nodes: CompressTreeNode[]) {
    const expanded: string[] = [];
    for (const node of nodes) {
      if (node.kind === "folder" && node.depth === 0) {
        expanded.push(node.path);
      }
    }
    return expanded;
  }

  function fileIconText(kind: FileKind, name = "") {
    if (kind === "word") return "W";
    if (kind === "excel") return "X";
    if (kind === "pdf") return "P";
    if (kind === "text") return "T";
    if (kind === "archive" || kind === "zip" || kind === "rar") return "";
    const extension = name.includes(".") ? name.split(".").pop() : "";
    return extension?.slice(0, 1).toUpperCase() || "F";
  }

  function archiveFormatOf(name: string) {
    const lower = name.toLowerCase();
    if (/\.part\d+\.rar$/.test(lower)) return "RAR 分卷";
    if (/\.z\d{2}$/.test(lower) || /\.zip\.\d{3}$/.test(lower)) return "ZIP 分卷";
    if (/\.7z\.\d{3}$/.test(lower)) return "7Z 分卷";
    if (/\.\d{3}$/.test(lower)) return "分卷";
    if (lower.endsWith(".tar.gz") || lower.endsWith(".tar.gzip") || lower.endsWith(".tgz")) return "TGZ";
    if (lower.endsWith(".tar.bz2") || lower.endsWith(".tar.bzip2") || lower.endsWith(".tbz") || lower.endsWith(".tbz2")) return "TBZ";
    if (lower.endsWith(".tar.xz") || lower.endsWith(".txz")) return "TXZ";
    if (lower.endsWith(".tar.zst") || lower.endsWith(".tar.zstd") || lower.endsWith(".tzst")) return "ZSTD";
    if (lower.endsWith(".tar.lzma2")) return "LZMA2";
    if (lower.endsWith(".tar.lzma") || lower.endsWith(".tlzma")) return "LZMA";
    if (lower.endsWith(".tar.lz4") || lower.endsWith(".tlz4")) return "LZ4";
    if (lower.endsWith(".tar.z")) return "Z";
    const extension = (name.split(".").pop() || "").toLowerCase();
    if (zipContainerExtensionPattern.test(lower)) return `${extension.toUpperCase()} / ZIP`;
    if (extension === "gzip") return "GZIP";
    if (extension === "bzip2") return "BZIP2";
    if (extension === "zst" || extension === "zstd") return "ZSTD";
    if (extension === "tgz") return "TGZ";
    if (extension === "tbz" || extension === "tbz2") return "TBZ";
    if (extension === "txz") return "TXZ";
    return extension.toUpperCase();
  }

  function compressExtension(format: string) {
    const extensionMap: Record<string, string> = {
      ZIP: "zip",
      "7Z": "7z",
      TAR: "tar",
      TGZ: "tgz",
      TBZ: "tbz",
      TXZ: "txz",
      GZ: "tar.gz",
      BZ2: "tar.bz2",
      XZ: "tar.xz",
      Z: "tar.Z",
      ZSTD: "tar.zst",
      LZMA: "tar.lzma",
      LZMA2: "tar.lzma2",
      LZ4: "tar.lz4",
    };
    return extensionMap[format] ?? format.toLowerCase();
  }

  function compressionLevelConfig(format: string) {
    const configs: Record<string, { min: number; max: number; defaultValue: number; low: string; high: string }> = {
      ZIP: { min: 0, max: 9, defaultValue: 6, low: "存储", high: "最佳" },
      "7Z": { min: 0, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      TGZ: { min: 1, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      GZ: { min: 1, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      TBZ: { min: 1, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      BZ2: { min: 1, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      TXZ: { min: 0, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      XZ: { min: 0, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      ZSTD: { min: 1, max: 22, defaultValue: 3, low: "最快", high: "最佳" },
      LZMA: { min: 0, max: 9, defaultValue: 6, low: "最快", high: "最佳" },
      LZ4: { min: 1, max: 2, defaultValue: 1, low: "最快", high: "更高" },
    };
    return configs[format] ?? null;
  }

  function supportsCompressionPassword(format: string) {
    return format === "ZIP" || format === "7Z";
  }

  function supportsSplitArchive(format: string) {
    return format === "ZIP" || format === "7Z";
  }

  function clampCompressionLevel(format: string, level: number) {
    const config = compressionLevelConfig(format);
    if (!config) return level;
    return Math.min(config.max, Math.max(config.min, Math.round(level)));
  }

  function defaultCompressionLevel(format: string) {
    return compressionLevelConfig(format)?.defaultValue ?? 6;
  }

  function applyDefaultCompressionFormat() {
    archiveFormat = defaultArchiveFormat;
    compressionLevel = defaultCompressionLevel(defaultArchiveFormat);
  }

  function enabledDefaultSaveDir() {
    return defaultSaveDirEnabled ? defaultSaveDir.trim() : "";
  }

  function enabledDefaultExtractDir() {
    return defaultExtractDirEnabled ? defaultExtractDir.trim() : "";
  }

  async function defaultOutputDirForPath(path: string) {
    if (!path || !isTauriRuntime()) return "";
    return await invoke<string>("default_output_dir", { path });
  }

  async function initialCompressSaveDir(path: string) {
    return enabledDefaultSaveDir() || (await defaultOutputDirForPath(path));
  }

  async function initialExtractDirForPath(path: string) {
    return enabledDefaultExtractDir() || (await defaultOutputDirForPath(path));
  }

  function configuredOperationDir(kind: DestinationPromptKind) {
    return kind === "compress" ? enabledDefaultSaveDir() : enabledDefaultExtractDir();
  }

  function destinationPromptCopy(kind: DestinationPromptKind) {
    return kind === "compress"
      ? {
          title: "选择压缩保存位置",
          description: "本次压缩包保存到哪里？",
        }
      : {
          title: "选择解压保存位置",
          description: "本次解压内容保存到哪里？",
        };
  }

  async function requestOperationDestination(kind: DestinationPromptKind, currentPath: string, preferredPath = "") {
    const configuredDir = configuredOperationDir(kind);
    if (configuredDir) return configuredDir;

    const cleanCurrentPath = currentPath.trim();
    const cleanPreferredPath = preferredPath.trim();
    const copy = destinationPromptCopy(kind);

    destinationPrompt = {
      kind,
      title: copy.title,
      description: copy.description,
      currentPath: cleanCurrentPath,
    };
    destinationPromptMode = "current";
    destinationPromptCustomPath = cleanPreferredPath || cleanCurrentPath;
    destinationPromptMessage = "";

    return await new Promise<string | null>((resolve) => {
      destinationPromptResolver = resolve;
    });
  }

  function clearDestinationPrompt() {
    destinationPrompt = null;
    destinationPromptMode = "current";
    destinationPromptCustomPath = "";
    destinationPromptMessage = "";
    destinationPromptResolver = null;
  }

  function cancelDestinationPrompt() {
    const resolve = destinationPromptResolver;
    clearDestinationPrompt();
    resolve?.(null);
  }

  function submitDestinationPrompt() {
    const prompt = destinationPrompt;
    if (!prompt) return;

    const selectedPath = destinationPromptMode === "current" ? prompt.currentPath.trim() : destinationPromptCustomPath.trim();
    if (!selectedPath) {
      destinationPromptMessage = "请选择保存位置。";
      return;
    }

    const resolve = destinationPromptResolver;
    clearDestinationPrompt();
    resolve?.(selectedPath);
  }

  async function browseDestinationPrompt() {
    const prompt = destinationPrompt;
    if (!prompt) return;

    const selected = await open({
      directory: true,
      multiple: false,
      canCreateDirectories: true,
      defaultPath: destinationPromptCustomPath || prompt.currentPath || undefined,
    });
    if (typeof selected === "string") {
      destinationPromptMode = "custom";
      destinationPromptCustomPath = selected;
    }
  }

  function isArchivePath(path: string) {
    return archiveExtensionPattern.test(path);
  }

  function extractPasswordParam() {
    const password = extractPassword.trim();
    return password ? password : null;
  }

  function isArchivePasswordError(error: unknown) {
    const message = String(error).toLowerCase();
    if (!message) return false;

    const directMatches = [
      "password",
      "passphrase",
      "wrong pass",
      "incorrect pass",
      "bad pass",
      "密码",
      "口令",
    ];
    if (directMatches.some((item) => message.includes(item))) return true;

    return (
      (message.includes("decrypt") || message.includes("encrypted") || message.includes("加密") || message.includes("解密")) &&
      !message.includes("unsupported")
    );
  }

  function passwordPromptTarget(action: ArchivePasswordAction) {
    if (action.kind === "open") return action.path.split(/[\\/]/).pop() || action.path;
    if (action.kind === "extract") return action.taskName;
    if (action.kind === "batchExtract") return action.taskName;
    if (action.kind === "testArchive") return archiveInfo?.name ?? "压缩包测试";
    if (action.kind === "editArchive") return action.label;
    if (action.kind === "drag") return action.rows.length > 1 ? `所选 ${action.rows.length} 个项目` : action.rows[0]?.name ?? "拖出项目";
    if (action.kind === "openWith" || action.kind === "openWithSuggestions") return action.row.name;
    return action.row.name;
  }

  function requestArchivePassword(action: ArchivePasswordAction, message?: string) {
    stopExtractProgressPolling();
    busy = false;
    extractTask = null;
    errorMessage = "";
    extractPassword = "";
    passwordPromptAction = action;
    passwordPromptValue = "";
    passwordPromptMessage = message || "请输入解压密码。";
  }

  function cancelArchivePasswordPrompt() {
    passwordPromptAction = null;
    passwordPromptValue = "";
    passwordPromptMessage = "";
    activePasswordRetryAction = null;
    busy = false;
  }

  async function retryArchivePasswordAction(action: ArchivePasswordAction) {
    if (action.kind === "open") {
      await openArchive(action.path);
    } else if (action.kind === "extract") {
      await startArchiveExtractTask(
        action.entries,
        action.outputDir,
        action.taskName,
        action.selectionCount,
        action.totalItems,
        action.totalBytes,
        action.conflictStrategy
      );
    } else if (action.kind === "batchExtract") {
      await startBatchExtractTask(action.paths, action.outputDir, action.taskName, action.selectionCount, action.conflictStrategy);
    } else if (action.kind === "preview") {
      await previewArchiveRow(action.row);
    } else if (action.kind === "openWith") {
      await openArchiveRowWithApplication(action.row, action.applicationPath);
    } else if (action.kind === "openWithSuggestions") {
      await loadContextOpenWithApps(action.row);
    } else if (action.kind === "drag") {
      await dragArchiveRowsOut(action.rows);
    } else if (action.kind === "testArchive") {
      await testCurrentArchiveIntegrity();
    } else {
      await runArchiveEdit(action.draft, action.label);
    }
  }

  async function submitArchivePasswordPrompt() {
    const action = passwordPromptAction;
    const password = passwordPromptValue.trim();
    if (!action) return;
    if (!password) {
      passwordPromptMessage = "请输入解压密码。";
      return;
    }

    extractPassword = password;
    passwordPromptAction = null;
    passwordPromptValue = "";
    passwordPromptMessage = "";
    await retryArchivePasswordAction(action);
  }

  function clearArchiveState() {
    archiveInfo = null;
    archiveTree = [];
    archiveSearchQuery = "";
    archiveKindFilter = "all";
    archiveIntegrityMessage = "";
    archiveInfoPanelOpen = false;
    expandedArchiveFolders = [];
    selectedArchivePaths = [];
    previewCache = new Map();
    handledCompletedTaskId = "";
  }

  $effect(() => {
    const config = compressionLevelConfig(archiveFormat);
    if (config) {
      const nextLevel = clampCompressionLevel(archiveFormat, compressionLevel);
      if (nextLevel !== compressionLevel) compressionLevel = nextLevel;
    }
    if (!supportsCompressionPassword(archiveFormat) && compressionPassword) {
      compressionPassword = "";
    }
    if (!supportsSplitArchive(archiveFormat) && splitArchive) {
      splitArchive = false;
    }
    const nextDictionary = Math.min(1024, Math.max(1, Math.round(sevenZipDictionarySizeMb || 1)));
    if (nextDictionary !== sevenZipDictionarySizeMb) sevenZipDictionarySizeMb = nextDictionary;
    const nextThreads = Math.min(32, Math.max(1, Math.round(compressThreads || 1)));
    if (nextThreads !== compressThreads) compressThreads = nextThreads;
  });

  $effect(() => {
    recentFiles;
    defaultArchiveFormat;
    defaultSaveDirEnabled;
    defaultExtractDirEnabled;
    defaultSaveDir;
    defaultExtractDir;
    skipDsStore;
    skipMacosMetadata;
    testAfterCompress;
    batchCompressQueue;
    sevenZipDictionarySizeMb;
    sevenZipSolid;
    compressThreads;
    sevenZipMethod;
    archiveColumnWidths;
    archiveSort;
    extractConflictStrategy;
    permissionGuideCompleted;
    savePackoPreferences();
  });

  $effect(() => {
    if (view !== "extract") return;
    queueSystemIconRequests(visibleArchiveRows().map((row) => systemIconRequest(row.kind, row.name)));
  });

  $effect(() => {
    if (view !== "compress") return;
    queueSystemIconRequests(flattenCompressTree().map((item) => systemIconRequest(item.kind, item.name, item.path)));
  });

  $effect(() => {
    queueSystemIconRequests(recentFiles.map((file) => systemIconRequest(file.kind, file.name, file.path)));
  });

  function defaultNameFromPath(path: string) {
    const name = path.split(/[\\/]/).pop() || "未命名";
    return name.replace(archiveSuffixPattern, "");
  }

  function archiveParentPath(path: string) {
    const normalized = path.replace(/\\/g, "/").replace(/\/+$/, "");
    const parts = normalized.split("/");
    if (parts.length <= 1) return "";
    return parts.slice(0, -1).join("/");
  }

  function archiveEntryIsHidden(path: string) {
    return path.replace(/\\/g, "/").split("/").some((part) => part.startsWith(".") && part.length > 1);
  }

  function totalSelectedSize() {
    return allCompressRows()
      .filter((item) => item.kind !== "folder")
      .reduce((total, item) => total + item.sizeValue, 0);
  }

  function formatSize(size: number) {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let value = size;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return unit === 0 ? `${size} ${units[unit]}` : `${value.toFixed(1)} ${units[unit]}`;
  }

  function currentRecentTimeLabel() {
    return new Date().toISOString().slice(0, 16).replace("T", " ");
  }

  function addRecentArchivePath(path: string, details?: Partial<RecentFile>) {
    if (!isArchivePath(path)) return;
    const next = {
      path,
      name: details?.name ?? path.split(/[\\/]/).pop() ?? path,
      format: archiveFormatOf(path),
      time: details?.time ?? currentRecentTimeLabel(),
      kind: "archive" as FileKind,
    };
    recentFiles = [next, ...recentFiles.filter((item) => item.path !== path)].slice(0, 8);
  }

  function setRecentFromArchive(info: ArchiveInfo) {
    addRecentArchivePath(info.path, {
      name: info.name,
    });
  }

  function uniqueArchivePaths(paths: string[]) {
    return Array.from(new Set(paths.filter(isArchivePath).map(normalizeLocalPath)));
  }

  async function pickArchive() {
    clearStatus();
    const selected = await open({
      multiple: true,
      fileAccessMode: "scoped",
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    const normalizedPaths = paths.map(normalizeLocalPath);
    if (isMainLauncherWindow()) {
      if (normalizedPaths.length === 1) {
        await openExtractWorkspace(normalizedPaths[0]);
      } else if (normalizedPaths.length > 1) {
        await openCompressWorkspace(normalizedPaths);
      }
      return;
    }
    if (normalizedPaths.length === 1) {
      await openArchive(normalizedPaths[0]);
    } else if (normalizedPaths.length > 1) {
      await startBatchExtractFlow(normalizedPaths);
    }
  }

  async function openArchive(path: string, keepPassword = false): Promise<OpenArchiveResult> {
    clearStatus();
    busy = true;
    const isDifferentArchive = path !== pendingArchivePath && path !== archiveInfo?.path;
    if (isDifferentArchive && !keepPassword) extractPassword = "";
    pendingArchivePath = path;
    clearArchiveState();
    try {
      const info = await invoke<ArchiveInfo>("list_archive", { path, password: extractPasswordParam() });
      archiveInfo = info;
      archiveTree = buildArchiveTree(info.entries);
      expandedArchiveFolders = defaultExpandedFolders(archiveTree);
      selectedArchivePaths = [];
      previewCache = new Map();
      handledCompletedTaskId = "";
      pendingArchivePath = info.path;
      extractDir = enabledDefaultExtractDir() || info.output_dir;
      setRecentFromArchive(info);
      view = "extract";
      return { ok: true };
    } catch (error) {
      const detail = String(error);
      if (isArchivePasswordError(detail)) {
        requestArchivePassword({ kind: "open", path }, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return { ok: false, passwordPrompt: true, error: detail };
      } else {
        errorMessage = detail;
        return { ok: false, error: detail };
      }
    } finally {
      view = "extract";
      busy = false;
    }
  }

  async function pickExtractDestination(defaultPath = extractDir) {
    const selected = await open({
      directory: true,
      multiple: false,
      canCreateDirectories: true,
      defaultPath: defaultPath || undefined,
    });
    return typeof selected === "string" ? selected : null;
  }

  async function chooseDefaultExtractDir() {
    const selected = await pickExtractDestination(defaultExtractDir || extractDir);
    if (selected) defaultExtractDir = selected;
  }

  async function startBatchExtractFlow(paths: string[]) {
    const archivePaths = Array.from(new Set(paths.map(normalizeLocalPath).filter(Boolean)));
    if (archivePaths.length === 0) return;
    for (const path of archivePaths) {
      addRecentArchivePath(path);
    }

    const currentDir = await defaultOutputDirForPath(archivePaths[0]);
    const outputDir = await requestOperationDestination("extract", currentDir || extractDir, extractDir || currentDir);
    if (!outputDir) return;
    extractPassword = "";
    extractDir = outputDir;
    clearArchiveState();
    view = "extract";
    const taskName = `批量解压 ${archivePaths.length} 个压缩包`;
    await startBatchExtractTask(archivePaths, outputDir, taskName, archivePaths.length);
  }

  function archiveRowByteSize(path: string) {
    return archiveInfo?.entries.find((entry) => entry.path === path)?.size ?? 1;
  }

  async function startBatchExtractTask(
    paths: string[],
    outputDir: string,
    taskName: string,
    selectionCount: number,
    conflictStrategy = extractConflictStrategy
  ) {
    const retryAction: ArchivePasswordAction = {
      kind: "batchExtract",
      paths: [...paths],
      outputDir,
      taskName,
      selectionCount,
      conflictStrategy,
    };
    activePasswordRetryAction = retryAction;
    busy = true;
    stopExtractProgressPolling();
    extractTaskMode = "extract";
    previewTaskEntryPath = "";
    handledCompletedTaskId = "";
    extractTaskArchiveName = taskName;
    extractTaskSelectionCount = selectionCount;
    extractTask = {
      task_id: "",
      status: "running",
      total: Math.max(selectionCount, 1),
      completed: 0,
      total_bytes: 1,
      completed_bytes: 0,
      current_bytes: 0,
      current_total_bytes: 1,
      current_item: "",
      output_path: outputDir,
      message: "正在创建批量解压任务。",
      error: null,
    };

    try {
      const progress = await invoke<ExtractTaskProgress>("start_batch_extract_task", {
        paths,
        outputDir,
        password: extractPasswordParam(),
        conflictStrategy,
      });
      applyExtractProgress(progress);
      if (!finishedExtractStatuses.has(progress.status)) {
        startExtractProgressPolling(progress.task_id);
      }
    } catch (error) {
      const message = String(error);
      if (isArchivePasswordError(message)) {
        activePasswordRetryAction = null;
        requestArchivePassword(retryAction, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return;
      }
      activePasswordRetryAction = null;
      extractTask = {
        task_id: "",
        status: "failed",
        total: 1,
        completed: 0,
        total_bytes: 1,
        completed_bytes: 0,
        current_bytes: 0,
        current_total_bytes: 1,
        current_item: "",
        output_path: outputDir,
        message,
        error: message,
      };
      busy = false;
    }
  }

  async function startArchiveExtractTask(
    entries: string[],
    outputDir: string,
    taskName: string,
    selectionCount: number,
    totalItems: number,
    totalBytes: number,
    conflictStrategy = extractConflictStrategy
  ) {
    if (!archiveInfo) return;
    const retryAction: ArchivePasswordAction = {
      kind: "extract",
      entries: [...entries],
      outputDir,
      taskName,
      selectionCount,
      totalItems,
      totalBytes,
      conflictStrategy,
    };
    activePasswordRetryAction = retryAction;
    busy = true;
    stopExtractProgressPolling();
    extractTaskMode = "extract";
    previewTaskEntryPath = "";
    handledCompletedTaskId = "";
    extractTaskArchiveName = taskName;
    extractTaskSelectionCount = selectionCount;
    extractTask = {
      task_id: "",
      status: "running",
      total: Math.max(totalItems, 1),
      completed: 0,
      total_bytes: Math.max(totalBytes, 1),
      completed_bytes: 0,
      current_bytes: 0,
      current_total_bytes: 1,
      current_item: "",
      output_path: outputDir,
      message: "正在创建解压任务。",
      error: null,
    };

    try {
      const progress = await invoke<ExtractTaskProgress>("start_extract_task", {
        path: archiveInfo.path,
        outputDir,
        entries,
        password: extractPasswordParam(),
        conflictStrategy,
      });
      applyExtractProgress(progress);
      if (!finishedExtractStatuses.has(progress.status)) {
        startExtractProgressPolling(progress.task_id);
      }
    } catch (error) {
      const message = String(error);
      if (isArchivePasswordError(message)) {
        activePasswordRetryAction = null;
        requestArchivePassword(retryAction, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return;
      }
      activePasswordRetryAction = null;
      extractTask = {
        task_id: "",
        status: "failed",
        total: 1,
        completed: 0,
        total_bytes: 1,
        completed_bytes: 0,
        current_bytes: 0,
        current_total_bytes: 1,
        current_item: "",
        output_path: outputDir,
        message,
        error: message,
      };
      busy = false;
    }
  }

  async function extractCurrentArchive() {
    clearStatus();
    if (!archiveInfo) {
      extractTask = {
        task_id: "",
        status: "failed",
        total: 1,
        completed: 0,
        total_bytes: 1,
        completed_bytes: 0,
        current_bytes: 0,
        current_total_bytes: 1,
        current_item: "",
        output_path: enabledDefaultExtractDir() || extractDir,
        message: "请先选择压缩包。",
        error: "请先选择压缩包。",
      };
      return;
    }

    const currentDir = archiveInfo.output_dir || (await defaultOutputDirForPath(archiveInfo.path));
    const outputDir = await requestOperationDestination("extract", currentDir || extractDir, extractDir || currentDir);
    if (!outputDir) return;
    extractDir = outputDir;

    const selectedRows = selectedArchiveEntries();
    const entries = selectedRows.map((row) => row.path);
    const totalBytes = entries.length > 0
      ? selectedRows.reduce((total, row) => total + archiveRowByteSize(row.path), 0)
      : archiveInfo.size;
    await startArchiveExtractTask(
      entries,
      outputDir,
      archiveInfo.name,
      entries.length,
      entries.length || archiveInfo.file_count,
      totalBytes
    );
  }

  async function pauseExtractTask() {
    if (extractTaskMode === "edit") return;
    if (!extractTask?.task_id) return;
    const command = extractTaskMode === "compress" ? "pause_compress_task" : "pause_extract_task";
    const progress = await invoke<ExtractTaskProgress>(command, { taskId: extractTask.task_id });
    applyExtractProgress(progress);
  }

  async function resumeExtractTask() {
    if (extractTaskMode === "edit") return;
    if (!extractTask?.task_id) return;
    const command = extractTaskMode === "compress" ? "resume_compress_task" : "resume_extract_task";
    const progress = await invoke<ExtractTaskProgress>(command, { taskId: extractTask.task_id });
    applyExtractProgress(progress);
    if (!finishedExtractStatuses.has(progress.status)) {
      startExtractProgressPolling(progress.task_id);
    }
  }

  async function cancelExtractTask() {
    if (extractTaskMode === "edit") return;
    if (!extractTask?.task_id) {
      extractTask = null;
      busy = false;
      return;
    }
    const command = extractTaskMode === "compress" ? "cancel_compress_task" : "cancel_extract_task";
    const progress = await invoke<ExtractTaskProgress>(command, { taskId: extractTask.task_id });
    applyExtractProgress(progress);
  }

  function closeExtractProgress() {
    if (isExtractTaskActive()) return;
    extractTask = null;
  }

  async function openExtractOutput() {
    if (!extractTask?.output_path) return;
    if (extractTaskMode === "preview") {
      await openPath(extractTask.output_path);
      return;
    }
    await revealItemInDir(extractTask.output_path);
  }

  async function previewArchiveRow(row: ArchiveRow) {
    clearStatus();
    if (!archiveInfo) {
      errorMessage = "请先选择压缩包。";
      return;
    }
    setRecentFromArchive(archiveInfo);
    if (row.kind === "folder") {
      toggleArchiveFolder(row.path);
      return;
    }

    const cached = previewCache.get(row.path);
    if (cached) {
      await openPath(cached);
      return;
    }

    const retryAction: ArchivePasswordAction = { kind: "preview", row };
    activePasswordRetryAction = retryAction;
    busy = true;
    stopExtractProgressPolling();
    extractTaskMode = "preview";
    previewTaskEntryPath = row.path;
    extractTaskArchiveName = row.name;
    extractTaskSelectionCount = 1;
    handledCompletedTaskId = "";
    extractTask = {
      task_id: "",
      status: "running",
      total: 1,
      completed: 0,
      total_bytes: Math.max(row.size === "-" ? 1 : Number.parseInt(row.size, 10) || 1, 1),
      completed_bytes: 0,
      current_bytes: 0,
      current_total_bytes: 1,
      current_item: row.path,
      output_path: "",
      message: "正在准备文件预览。",
      error: null,
    };

    try {
      const progress = await invoke<ExtractTaskProgress>("start_preview_task", {
        path: archiveInfo.path,
        entryPath: row.path,
        password: extractPasswordParam(),
      });
      applyExtractProgress(progress);
      if (!finishedExtractStatuses.has(progress.status)) {
        startExtractProgressPolling(progress.task_id);
      }
    } catch (error) {
      const message = String(error);
      if (isArchivePasswordError(message)) {
        activePasswordRetryAction = null;
        requestArchivePassword(retryAction, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return;
      }
      activePasswordRetryAction = null;
      extractTask = {
        task_id: "",
        status: "failed",
        total: 1,
        completed: 0,
        total_bytes: 1,
        completed_bytes: 0,
        current_bytes: 0,
        current_total_bytes: 1,
        current_item: row.path,
        output_path: "",
        message,
        error: message,
      };
      busy = false;
    }
  }

  function startArchiveFileDrag(row: ArchiveRow, event: MouseEvent) {
    if (event.button !== 0 || busy || !archiveInfo) return;
    const target = event.target as HTMLElement | null;
    const shouldPromiseDrag = Boolean(target?.closest(".archive-file-drag-handle, .file-badge, .folder-icon, .system-file-icon"));
    archiveFileDragCandidate = {
      row,
      rows: archiveDragSourceRows(row),
      mode: shouldPromiseDrag ? "promise" : "move",
      startX: event.clientX,
      startY: event.clientY,
      started: false,
      promiseStarted: false,
    };
    archiveFileDragStarted = false;
    archiveInternalDropTargetPath = null;
  }

  function archivePointInsideList(clientX: number, clientY: number) {
    const list = document.querySelector<HTMLElement>(".archive-list-panel");
    const rect = list?.getBoundingClientRect();
    return Boolean(rect && clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom);
  }

  function shouldStartArchivePromiseDrag(candidate: ArchiveFileDragCandidate, event: MouseEvent) {
    if (candidate.rows.some((row) => row.kind === "folder")) return true;
    return candidate.mode === "promise" || !archivePointInsideList(event.clientX, event.clientY);
  }

  function beginArchivePromiseDrag(candidate: ArchiveFileDragCandidate) {
    if (candidate.promiseStarted) return;
    candidate.mode = "promise";
    candidate.promiseStarted = true;
    archiveInternalDropTargetPath = null;
    void dragArchiveRowsOut(candidate.rows);
  }

  function moveArchiveFileDrag(event: MouseEvent) {
    const candidate = archiveFileDragCandidate;
    if (!candidate) return;
    if (candidate.started) {
      event.preventDefault();
      if (shouldStartArchivePromiseDrag(candidate, event)) {
        beginArchivePromiseDrag(candidate);
        return;
      }
      if (candidate.mode === "move") {
        setArchiveInternalDropTargetFromPoint(event.clientX, event.clientY);
      }
      return;
    }

    const distance = Math.hypot(event.clientX - candidate.startX, event.clientY - candidate.startY);
    if (distance < 6) return;

    candidate.started = true;
    archiveFileDragStarted = true;
    event.preventDefault();

    if (shouldStartArchivePromiseDrag(candidate, event)) {
      beginArchivePromiseDrag(candidate);
      return;
    }

    setArchiveInternalDropTargetFromPoint(event.clientX, event.clientY);
  }

  function handleWindowMouseMove(event: MouseEvent) {
    if (moveArchiveColumnResize(event)) return;
    moveArchiveFileDrag(event);
  }

  function stopArchiveFileDrag(resetDelay = 0) {
    const candidate = archiveFileDragCandidate;
    const targetPath = archiveInternalDropTargetPath;
    const shouldMove = Boolean(candidate?.mode === "move" && candidate.started && targetPath !== null && isArchiveMoveTargetValid(candidate.rows, targetPath));
    archiveFileDragCandidate = null;
    archiveInternalDropTargetPath = null;
    window.setTimeout(() => {
      archiveFileDragStarted = false;
    }, resetDelay);

    if (candidate && targetPath !== null && shouldMove) {
      void moveArchiveRowsToFolder(candidate.rows, targetPath);
    }
  }

  function stopArchivePointerInteractions() {
    stopArchiveColumnResize();
    stopArchiveRowSelection();
    stopCompressRowSelection();
    stopArchiveFileDrag();
  }

  function uniqueArchiveDragRows(rows: ArchiveRow[]) {
    const unique = new Map<string, ArchiveRow>();
    for (const row of rows) {
      if (!row.path) continue;
      unique.set(`${row.kind}:${row.path}`, row);
    }
    return Array.from(unique.values());
  }

  async function dragArchiveRowsOut(rows: ArchiveRow[]) {
    clearStatus();
    if (!archiveInfo) return;
    const dragRows = uniqueArchiveDragRows(rows);
    if (dragRows.length === 0) return;
    if (dragRows.some((row) => row.isUnsafePath)) {
      errorMessage = "包含不安全路径，已阻止拖出。";
      stopArchiveFileDrag(250);
      return;
    }
    if (!isTauriRuntime()) {
      errorMessage = "当前环境不支持原生文件拖出，请在 Tauri 桌面窗口中使用。";
      return;
    }

    try {
      const pendingMessage = dragRows.length > 1 ? `拖到 Finder 后松开，${dragRows.length} 个项目会写入目标位置。` : "拖到 Finder 后松开，项目会写入目标位置。";
      statusMessage = pendingMessage;
      archivePromiseDragProgress = {
        dragId: `pending-${Date.now()}`,
        status: "running",
        total: dragRows.length,
        completed: 0,
        currentItem: dragRows.length > 1 ? `所选 ${dragRows.length} 个项目` : dragRows[0].name,
        message: pendingMessage,
        error: null,
      };
      scheduleArchivePromiseDragHide(4500, archivePromiseDragProgress.dragId);
      await invoke("start_archive_entries_promise_drag", {
        path: archiveInfo.path,
        items: dragRows.map((row) => ({
          entryPath: row.path,
          promisedName: row.name,
          isDir: row.kind === "folder",
        })),
        password: extractPasswordParam(),
      });
    } catch (error) {
      const message = String(error);
      if (isArchivePasswordError(message)) {
        archivePromiseDragProgress = null;
        requestArchivePassword({ kind: "drag", rows: dragRows }, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
      } else {
        archivePromiseDragProgress = {
          dragId: `failed-${Date.now()}`,
          status: "failed",
          total: dragRows.length,
          completed: 0,
          currentItem: dragRows.length > 1 ? `所选 ${dragRows.length} 个项目` : dragRows[0].name,
          message,
          error: message,
        };
        scheduleArchivePromiseDragHide(6000, archivePromiseDragProgress.dragId);
        errorMessage = message;
      }
    } finally {
      stopArchiveFileDrag(250);
    }
  }

  async function testCurrentArchiveIntegrity() {
    clearStatus();
    if (!archiveInfo) return;
    const retryAction: ArchivePasswordAction = { kind: "testArchive" };
    activePasswordRetryAction = retryAction;
    busy = true;
    archiveIntegrityMessage = "正在测试...";
    try {
      const result = await invoke<OperationResult>("test_archive_integrity", {
        path: archiveInfo.path,
        password: extractPasswordParam(),
      });
      archiveIntegrityMessage = result.message;
      statusMessage = result.message;
      activePasswordRetryAction = null;
    } catch (error) {
      const message = String(error);
      archiveIntegrityMessage = "";
      if (isArchivePasswordError(message)) {
        activePasswordRetryAction = null;
        requestArchivePassword(retryAction, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return;
      }
      activePasswordRetryAction = null;
      errorMessage = message;
    } finally {
      busy = false;
    }
  }

  async function clearArchivePreviewCache() {
    clearStatus();
    try {
      const result = await invoke<OperationResult>("clear_preview_cache", {
        path: archiveInfo?.path ?? null,
      });
      previewCache = new Map();
      statusMessage = result.message;
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function runArchiveEdit(draft: ArchiveEditDraft, label: string) {
    clearStatus();
    if (!archiveInfo) return false;
    const retryAction: ArchivePasswordAction = { kind: "editArchive", draft, label };
    const previousExpandedFolders = [...expandedArchiveFolders];
    activePasswordRetryAction = retryAction;
    startArchiveEditProgress(label);
    busy = true;
    try {
      const result = await invoke<OperationResult>("edit_archive", {
        options: {
          path: archiveInfo.path,
          password: extractPasswordParam(),
          deleteEntries: draft.deleteEntries ?? [],
          renameEntries: draft.renameEntries ?? [],
          addPaths: draft.addPaths ?? [],
          addEntries: draft.addEntries ?? [],
          createDirs: draft.createDirs ?? [],
          replaceEntries: draft.replaceEntries ?? [],
          outputPath: draft.outputPath ?? null,
        },
      });
      activePasswordRetryAction = null;
      const nextPath = result.output_path;
      if (extractTaskMode === "edit" && extractTask?.status === "running") {
        extractTask = {
          ...extractTask,
          completed: 4,
          completed_bytes: 96,
          current_bytes: 96,
          current_item: "刷新文件树",
          message: "正在刷新文件树。",
        };
      }
      await openArchive(nextPath, true);
      expandedArchiveFolders = restoredArchiveExpandedFolders(previousExpandedFolders, draft);
      statusMessage = `${label}已保存。`;
      addRecentArchivePath(nextPath);
      finishArchiveEditProgress(nextPath, `${label}已保存。`);
      return true;
    } catch (error) {
      const message = String(error);
      if (isArchivePasswordError(message)) {
        stopArchiveEditProgressTimer();
        extractTask = null;
        activePasswordRetryAction = null;
        requestArchivePassword(retryAction, extractPasswordParam() ? "密码不正确，请重新输入。" : "请输入解压密码。");
        return false;
      }
      activePasswordRetryAction = null;
      errorMessage = message;
      failArchiveEditProgress(message);
      return false;
    } finally {
      if (extractTaskMode !== "edit" || extractTask?.status !== "running") {
        busy = false;
      }
    }
  }

  async function addItemsToArchive() {
    if (!archiveInfo) return;
    const targetDir = archiveContextTargetDir();
    closeArchiveContextMenu();
    clearStatus();
    let paths: string[] = [];
    if (isTauriRuntime()) {
      paths = await invoke<string[]>("pick_compress_sources");
    } else {
      const selected = await open({ multiple: true, fileAccessMode: "scoped" });
      paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    }
    if (paths.length === 0) return;
    await runArchiveEdit(
      targetDir ? { addEntries: paths.map((path) => ({ sourcePath: path, targetDir })) } : { addPaths: paths },
      "添加"
    );
  }

  async function addDroppedItemsToArchive(paths: string[], targetDir = "") {
    if (!archiveInfo || busy || paths.length === 0) return;
    clearStatus();
    closeArchiveContextMenu();
    const cleanTargetDir = targetDir.trim();
    await runArchiveEdit(
      cleanTargetDir ? { addEntries: paths.map((path) => ({ sourcePath: path, targetDir: cleanTargetDir })) } : { addPaths: paths },
      "添加"
    );
  }

  async function saveArchiveAs() {
    if (!archiveInfo) return;
    closeArchiveContextMenu();
    const selected = await save({
      defaultPath: archiveInfo.path,
      filters: [{ name: "压缩包", extensions: archiveExtensions }],
    });
    if (!selected) return;
    await runArchiveEdit({ outputPath: selected }, "另存为");
  }

  async function pickCompressFiles() {
    clearStatus();
    const selected = await open({ multiple: true, fileAccessMode: "scoped" });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    await addCompressPaths(paths);
  }

  async function pickCompressSources() {
    clearStatus();
    if (!isTauriRuntime()) {
      await pickCompressFiles();
      return;
    }

    try {
      const paths = await invoke<string[]>("pick_compress_sources");
      if (isMainLauncherWindow()) {
        await openCompressWorkspace(paths);
        return;
      }
      await addCompressPaths(paths);
    } catch (error) {
      errorMessage = String(error);
    }
  }

  async function addCompressPaths(paths: string[]) {
    if (paths.length === 0) return;
    const startsNewCompressTask = selectedItems.length === 0;
    busy = true;
    try {
      if (startsNewCompressTask) {
        applyDefaultCompressionFormat();
        saveDir = enabledDefaultSaveDir();
      }

      const incoming = paths.map(normalizeLocalPath);
      const currentRoots = selectedItems.map((item) => normalizeLocalPath(item.path));
      let nextRootPaths = [...currentRoots];
      let nextExcluded = excludedCompressPaths.map(normalizeLocalPath);

      for (const path of incoming) {
        const containingRoot = nextRootPaths.find((root) => pathContains(root, path));
        if (containingRoot) {
          nextExcluded = nextExcluded.filter((excluded) => !pathContains(path, excluded) && excluded !== path);
          continue;
        }

        nextRootPaths = nextRootPaths.filter((root) => !pathContains(path, root));
        nextExcluded = nextExcluded.filter((excluded) => !pathContains(path, excluded));
        nextRootPaths.push(path);
      }

      nextRootPaths = Array.from(new Set(nextRootPaths));
      const items = await invoke<FileInfo[]>("describe_paths", { paths: nextRootPaths });
      selectedItems = items.map(compressSourceFromInfo);
      excludedCompressPaths = nextExcluded;
      await refreshCompressTree(selectedItems, excludedCompressPaths);
      if (!archiveName && selectedItems.length > 0) {
        archiveName = defaultNameFromPath(selectedItems[0].path);
      }
      if (!saveDir && selectedItems.length > 0) {
        saveDir = await initialCompressSaveDir(selectedItems[0].path);
      }
      view = "compress";
    } catch (error) {
      errorMessage = String(error);
    } finally {
      busy = false;
    }
  }

  async function refreshCompressTree(roots = selectedItems, excluded = excludedCompressPaths) {
    if (roots.length === 0) {
      compressTree = [];
      expandedCompressFolders = [];
      selectedCompressPaths = [];
      return;
    }

    const items = await invoke<FileInfo[]>("describe_compress_paths", {
      paths: roots.map((item) => item.path),
      excludedPaths: excluded,
      skipDsStore,
      skipMacosMetadata,
    });
    compressTree = buildCompressTree(items);
    const existingExpanded = expandedCompressFolders.filter((path) => allCompressRows(compressTree).some((row) => row.path === path));
    expandedCompressFolders = Array.from(new Set([...existingExpanded, ...defaultExpandedCompressFolders(compressTree)]));
    selectedCompressPaths = selectedCompressPaths.filter((path) => allCompressRows(compressTree).some((row) => row.path === path));
  }

  async function removeSelectedCompressItems() {
    if (selectedCompressPaths.length === 0) return;
    const selected = selectedCompressPaths.map(normalizeLocalPath);
    const nextRoots = selectedItems.filter((item) => !selected.some((path) => pathContains(path, item.path)));
    const removedRootPaths = selectedItems
      .filter((item) => !nextRoots.some((next) => next.path === item.path))
      .map((item) => normalizeLocalPath(item.path));
    const nextExcluded = Array.from(new Set([
      ...excludedCompressPaths.filter((path) => !removedRootPaths.some((root) => pathContains(root, path))),
      ...selected.filter((path) => !removedRootPaths.some((root) => pathContains(root, path))),
    ]));

    selectedItems = nextRoots;
    excludedCompressPaths = nextExcluded;
    selectedCompressPaths = [];
    await refreshCompressTree(selectedItems, excludedCompressPaths);
    if (selectedItems.length === 0) {
      archiveName = "";
      saveDir = enabledDefaultSaveDir();
    }
  }

  async function setSkipDsStore(value: boolean) {
    skipDsStore = value;
    selectedCompressPaths = [];
    if (selectedItems.length === 0) return;

    busy = true;
    try {
      await refreshCompressTree(selectedItems, excludedCompressPaths);
    } catch (error) {
      errorMessage = String(error);
    } finally {
      busy = false;
    }
  }

  async function setSkipMacosMetadata(value: boolean) {
    skipMacosMetadata = value;
    selectedCompressPaths = [];
    if (selectedItems.length === 0) return;

    busy = true;
    try {
      await refreshCompressTree(selectedItems, excludedCompressPaths);
    } catch (error) {
      errorMessage = String(error);
    } finally {
      busy = false;
    }
  }

  function clearSelected() {
    selectedItems = [];
    compressTree = [];
    expandedCompressFolders = [];
    selectedCompressPaths = [];
    excludedCompressPaths = [];
    archiveName = "";
    saveDir = enabledDefaultSaveDir();
    clearStatus();
  }

  function resetCompressState() {
    selectedItems = [];
    compressTree = [];
    expandedCompressFolders = [];
    selectedCompressPaths = [];
    excludedCompressPaths = [];
    archiveName = "";
    saveDir = enabledDefaultSaveDir();
    applyDefaultCompressionFormat();
    compressionPassword = "";
    splitArchive = false;
    splitSizeMb = 100;
    clearStatus();
  }

  async function chooseCompressSaveDir() {
    const selected = await open({
      directory: true,
      multiple: false,
      canCreateDirectories: true,
      defaultPath: saveDir || undefined,
    });
    if (typeof selected === "string") {
      saveDir = selected;
    }
  }

  async function chooseDefaultSaveDir() {
    const selected = await open({
      directory: true,
      multiple: false,
      canCreateDirectories: true,
      defaultPath: defaultSaveDir || saveDir || undefined,
    });
    if (typeof selected === "string") {
      defaultSaveDir = selected;
    }
  }

  async function compressSelected() {
    clearStatus();
    if (selectedItems.length === 0) {
      errorMessage = "请先选择要压缩的文件或文件夹。";
      return;
    }
    if (allCompressRows().length === 0) {
      errorMessage = "没有可压缩的项目。";
      return;
    }
    const currentSaveDir = selectedItems[0] ? await defaultOutputDirForPath(selectedItems[0].path) : saveDir;
    const outputDir = await requestOperationDestination("compress", currentSaveDir || saveDir, saveDir || currentSaveDir);
    if (!outputDir) return;
    saveDir = outputDir;

    const levelConfig = compressionLevelConfig(archiveFormat);
    const taskOutput = `${outputDir.replace(/[\\/]$/, "")}/${archiveName || "未命名"}.${compressExtension(archiveFormat)}`;
    busy = true;
    stopExtractProgressPolling();
    extractTaskMode = "compress";
    previewTaskEntryPath = "";
    handledCompletedTaskId = "";
    extractTaskArchiveName = `${archiveName || "未命名"}.${compressExtension(archiveFormat)}`;
    extractTaskSelectionCount = allCompressRows().length || selectedItems.length;
    extractTask = {
      task_id: "",
      status: "running",
      total: Math.max(extractTaskSelectionCount, 1),
      completed: 0,
      total_bytes: Math.max(totalSelectedSize(), 1),
      completed_bytes: 0,
      current_bytes: 0,
      current_total_bytes: Math.max(totalSelectedSize(), 1),
      current_item: extractTaskArchiveName,
      output_path: taskOutput,
      message: "正在创建压缩任务。",
      error: null,
    };

    try {
      const progress = await invoke<ExtractTaskProgress>("start_compress_task", {
        options: {
          sourcePaths: selectedItems.map((item) => item.path),
          outputDir,
          archiveName,
          format: archiveFormat,
          compressionLevel: levelConfig ? clampCompressionLevel(archiveFormat, compressionLevel) : null,
          password: supportsCompressionPassword(archiveFormat) && compressionPassword.trim() ? compressionPassword : null,
          volumeSizeMb: supportsSplitArchive(archiveFormat) && splitArchive ? Math.max(1, Math.round(splitSizeMb)) : null,
          excludedPaths: excludedCompressPaths,
          skipDsStore,
          advanced: {
            batchQueue: batchCompressQueue && selectedItems.length > 1,
            dictionarySizeMb: archiveFormat === "7Z" ? Math.min(1024, Math.max(1, Math.round(sevenZipDictionarySizeMb))) : null,
            solid: archiveFormat === "7Z" ? sevenZipSolid : null,
            threads: archiveFormat === "7Z" ? Math.min(32, Math.max(1, Math.round(compressThreads))) : null,
            method: archiveFormat === "7Z" ? sevenZipMethod : null,
            testAfterCompress,
            skipMacosMetadata,
          },
        },
      });
      applyExtractProgress(progress);
      if (!finishedExtractStatuses.has(progress.status)) {
        startExtractProgressPolling(progress.task_id);
      }
    } catch (error) {
      const message = String(error);
      errorMessage = message;
      extractTask = {
        task_id: "",
        status: "failed",
        total: 1,
        completed: 0,
        total_bytes: 1,
        completed_bytes: 0,
        current_bytes: 0,
        current_total_bytes: 1,
        current_item: "",
        output_path: taskOutput,
        message,
        error: message,
      };
      busy = false;
    }
  }

  async function bootWorkspaceWindow() {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const queryWorkspace = params.get("workspace");
    const currentWindowLabel = isTauriRuntime() ? getCurrentWindow().label : "";
    const labelWorkspace: View | null = currentWindowLabel.startsWith("packo-extract-")
      ? "extract"
      : currentWindowLabel.startsWith("packo-compress-")
        ? "compress"
        : null;
    const workspace = queryWorkspace === "extract" || queryWorkspace === "compress"
      ? queryWorkspace
      : labelWorkspace;
    if (workspace !== "extract" && workspace !== "compress") {
      workspaceBootKind = null;
      return;
    }

    isWorkspaceWindow = Boolean(labelWorkspace) || queryWorkspace === "extract" || queryWorkspace === "compress";
    workspaceBootKind = workspace;
    if (!isTauriRuntime()) {
      view = workspace;
      workspaceBootKind = null;
      return;
    }

    try {
      const payload = await invoke<WorkWindowPayload | null>("get_work_window_payload");
      const paths = payload?.paths ?? [];
      if (payload?.kind === "extract" && paths.length > 0) {
        workspaceBootKind = "extract";
        pendingArchivePath = paths[0];
        await showPackoWindowWhenReady();
        const result = await openArchive(paths[0]);
        if (!result.ok && !result.passwordPrompt) {
          await closeInvalidArchiveWorkspace(paths[0], result.error);
          return;
        }
      } else if (payload?.kind === "compress" && paths.length > 0) {
        workspaceBootKind = "compress";
        await showPackoWindowWhenReady();
        await addCompressPaths(paths);
      } else {
        view = workspace;
        await showPackoWindowWhenReady();
      }
    } catch (error) {
      view = workspace;
      errorMessage = String(error);
      await showPackoWindowWhenReady();
    } finally {
      workspaceBootKind = null;
    }
  }

  onMount(() => {
    loadPackoPreferences();
    preferencesLoaded = true;
    savePackoPreferences();

    const handleStorage = (event: StorageEvent) => {
      if (event.key === packoStorageKey) syncRecentFilesFromStorage();
    };
    window.addEventListener("storage", handleStorage);
    void bootWorkspaceWindow().finally(async () => {
      await showPackoWindowWhenReady();
      showFirstLaunchPermissionGuide();
    });
    let disposed = false;
    let unlistenMenuAction: (() => void) | null = null;
    let unlistenArchivePromiseDragProgress: (() => void) | null = null;
    if (isTauriRuntime()) {
      void listen<PackoMenuAction>("packo-menu-action", (event) => {
        void handlePackoMenuAction(event.payload);
      }).then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlistenMenuAction = unlisten;
        }
      });
      void listen<ArchivePromiseDragProgress>("archive-promise-drag-progress", (event) => {
        handleArchivePromiseDragProgress(event.payload);
      }).then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlistenArchivePromiseDragProgress = unlisten;
        }
      });
    }

    const cleanup = () => {
      disposed = true;
      unlistenMenuAction?.();
      unlistenArchivePromiseDragProgress?.();
      window.removeEventListener("storage", handleStorage);
      stopExtractProgressPolling();
      stopArchiveEditProgressTimer();
      clearArchivePromiseDragHideTimer();
    };

    if (!isTauriRuntime()) {
      return cleanup;
    }
    getCurrentWebview().onDragDropEvent(async (event) => {
      if (event.payload.type === "leave") {
        archiveExternalDropTargetPath = null;
        return;
      }

      if ((event.payload.type === "enter" || event.payload.type === "over") && view === "extract" && archiveInfo) {
        setArchiveExternalDropTargetFromPosition(event.payload.position);
        return;
      }

      if (event.payload.type !== "drop") return;
      const archiveDropTargetPath = view === "extract" && archiveInfo ? archiveExternalDropTargetPath : null;
      setArchiveExternalDropTargetFromPosition(event.payload.position);
      const targetDir = archiveExternalDropTargetPath ?? archiveDropTargetPath ?? "";
      archiveExternalDropTargetPath = null;
      const paths = event.payload.paths.map(normalizeLocalPath);
      const first = paths[0] || "";
      const archivePaths = uniqueArchivePaths(paths);
      const allDroppedArchives = paths.length > 0 && paths.every(isArchivePath);

      if (view === "extract" && archiveInfo && paths.length > 0) {
        await addDroppedItemsToArchive(paths, targetDir);
        return;
      }

      for (const path of archivePaths) {
        addRecentArchivePath(path);
      }

      if (isMainLauncherWindow()) {
        if (paths.length === 1) {
          if (await shouldCompressSingleImportPath(first)) {
            await openCompressWorkspace(paths);
          } else {
            await openExtractWorkspace(first);
          }
        } else if (paths.length > 1) {
          await openCompressWorkspace(paths);
        }
      } else if (view === "compress") {
        await addCompressPaths(paths);
      } else if (paths.length === 1) {
        if (await shouldCompressSingleImportPath(first)) {
          await addCompressPaths(paths);
        } else {
          await openArchive(first);
        }
      } else if (archivePaths.length > 1 && allDroppedArchives) {
        await startBatchExtractFlow(archivePaths);
      } else {
        await addCompressPaths(paths);
      }
    });

    return cleanup;
  });
</script>

<svelte:head>
  <title>Packo</title>
</svelte:head>

<svelte:window
  onmousemove={handleWindowMouseMove}
  onmouseup={stopArchivePointerInteractions}
  onkeydown={handleGlobalKeydown}
  oncontextmenu={handleGlobalContextMenu}
/>

<main class="stage" class:compact={view !== "home" || workspaceBootKind !== null} class:resizing-columns={archiveColumnResize !== null}>
  <section class="app-shell" class:task-shell={view !== "home" || workspaceBootKind !== null}>
    {#if workspaceBootKind}
      <header class="titlebar task-titlebar" data-tauri-drag-region onmousedown={startWindowDrag} role="presentation">
        <div class="task-title">
          <strong>{workspaceBootKind === "compress" ? "压缩" : "打开压缩包"}</strong>
        </div>
      </header>

      <section class="workspace-loading" aria-live="polite" aria-busy="true">
        <span class="workspace-loading-icon">
          <LoaderCircle size={34} strokeWidth={1.8} />
        </span>
        <div>
          <strong>{workspaceBootTitle()}</strong>
          <span>{workspaceBootDescription()}</span>
        </div>
      </section>
    {:else if view === "home"}
      <header class="titlebar main-titlebar" data-tauri-drag-region onmousedown={startWindowDrag} role="presentation">
        <div class="app-tabs" data-no-drag role="tablist" aria-label="应用面板">
          <button
            type="button"
            class:active={activeAppPanel === "recent"}
            onclick={() => toggleAppPanel("recent")}
            aria-label="最近文件"
            title="最近文件"
          >
            <FolderOpen size={18} strokeWidth={1.9} />
            <span>最近文件</span>
          </button>
          <button
            type="button"
            class:active={activeAppPanel === "settings"}
            onclick={() => toggleAppPanel("settings")}
            aria-label="设置"
            title="设置"
          >
            <Settings size={18} strokeWidth={1.9} />
            <span>设置</span>
          </button>
        </div>
      </header>

      <div class="home-layout">
        <section class="home-content">
          <div class="home-center">
            <h1>欢迎使用Packo</h1>
            <div class="hero-actions">
              <button class="action-card" type="button" onclick={() => void openBlankCompressWorkspace()}>
                <span class="home-action-icon compress-icon">
                  <Archive size={54} strokeWidth={1.45} />
                </span>
                <span class="action-copy">
                  <strong>压缩</strong>
                </span>
              </button>

              <button class="action-card" type="button" onclick={pickArchive}>
                <span class="home-action-icon extract-icon">
                  <Download size={54} strokeWidth={1.45} />
                </span>
                <span class="action-copy">
                  <strong>解压</strong>
                </span>
              </button>
            </div>
          </div>
        </section>
      </div>
    {:else if view === "extract"}
      <header class="titlebar task-titlebar" data-tauri-drag-region onmousedown={startWindowDrag} role="presentation">
        <div class="task-title">
          <strong>打开压缩包</strong>
        </div>
        <button class="primary-button titlebar-action with-icon" type="button" onclick={extractCurrentArchive} disabled={!archiveInfo || busy} data-no-drag>
          <Download size={18} strokeWidth={1.9} />
          <span>{busy ? "处理中..." : selectedArchivePaths.length > 0 ? "解压所选" : "全部解压"}</span>
        </button>
      </header>

      <section class="extract-toolbar">
        <div class="archive-path">
          <strong>{archiveInfo?.name ?? "未选择压缩包"}</strong>
          <span>{archiveInfo?.path ?? (pendingArchivePath || "选择或拖入压缩包后查看内容")}</span>
        </div>
        <div class="archive-search-tools">
          <label class="archive-search-field" aria-label="搜索压缩包内容">
            <Search size={16} strokeWidth={2} />
            <input type="search" bind:value={archiveSearchQuery} placeholder="搜索包内文件" />
          </label>
          <select bind:value={archiveKindFilter} aria-label="过滤压缩包内容">
            <option value="all">全部</option>
            <option value="files">文件</option>
            <option value="folders">文件夹</option>
            <option value="encrypted">加密</option>
            <option value="hidden">隐藏</option>
            <option value="executable">可执行</option>
          </select>
          <button
            class="icon-button archive-info-button"
            type="button"
            aria-label="压缩包信息"
            title="压缩包信息"
            aria-pressed={archiveInfoPanelOpen}
            disabled={!archiveInfo}
            onclick={() => {
              activeAppPanel = null;
              archiveInfoPanelOpen = !archiveInfoPanelOpen;
            }}
          >
            <Info size={18} strokeWidth={2} />
          </button>
        </div>
      </section>

      <section class="extract-grid">
        <div
          class="archive-list-panel"
          class:archive-root-drop-target={archiveInternalDropTargetPath === "" || archiveExternalDropTargetPath === ""}
          aria-label="压缩包内容"
          role="grid"
          tabindex="0"
          onmouseup={stopArchiveRowSelection}
          onmouseleave={() => {
            stopArchiveRowSelection();
          }}
          ondragstart={(event) => event.preventDefault()}
          oncontextmenu={(event) => openArchiveContextMenu(null, event)}
        >
          <div class="archive-table-row archive-head" style={archiveTableColumnsStyle()}>
            <span class="check-cell">
              <input
                type="checkbox"
                aria-label="全选文件"
                checked={archiveSelectableRows().length > 0 && archiveSelectableRows().every((row) => selectedArchivePaths.includes(row.path))}
                disabled={archiveSelectableRows().length === 0}
                onchange={toggleAllArchiveRows}
              />
            </span>
            <span class="archive-head-cell">
              <button
                class="archive-sort-button"
                type="button"
                aria-label={archiveSortTitle("name")}
                title={archiveSortTitle("name")}
                onclick={() => setArchiveSort("name")}
              >
                <span>名称</span>
                {#if archiveSort.column === "name"}
                  <ChevronDown class={`sort-chevron ${archiveSort.direction === "asc" ? "ascending" : ""}`} size={14} />
                {/if}
              </button>
              <button class="column-resizer" type="button" aria-label="调整名称列宽" title="拖动调整列宽" onmousedown={(event) => startArchiveColumnResize("name", event)}></button>
            </span>
            <span class="archive-head-cell">
              <button
                class="archive-sort-button"
                type="button"
                aria-label={archiveSortTitle("type")}
                title={archiveSortTitle("type")}
                onclick={() => setArchiveSort("type")}
              >
                <span>类型</span>
                {#if archiveSort.column === "type"}
                  <ChevronDown class={`sort-chevron ${archiveSort.direction === "asc" ? "ascending" : ""}`} size={14} />
                {/if}
              </button>
              <button class="column-resizer" type="button" aria-label="调整类型列宽" title="拖动调整列宽" onmousedown={(event) => startArchiveColumnResize("type", event)}></button>
            </span>
            <span class="archive-head-cell">
              <button
                class="archive-sort-button"
                type="button"
                aria-label={archiveSortTitle("size")}
                title={archiveSortTitle("size")}
                onclick={() => setArchiveSort("size")}
              >
                <span>大小</span>
                {#if archiveSort.column === "size"}
                  <ChevronDown class={`sort-chevron ${archiveSort.direction === "asc" ? "ascending" : ""}`} size={14} />
                {/if}
              </button>
              <button class="column-resizer" type="button" aria-label="调整大小列宽" title="拖动调整列宽" onmousedown={(event) => startArchiveColumnResize("size", event)}></button>
            </span>
            <span class="archive-head-cell">
              <button
                class="archive-sort-button"
                type="button"
                aria-label={archiveSortTitle("modified")}
                title={archiveSortTitle("modified")}
                onclick={() => setArchiveSort("modified")}
              >
                <span>修改时间</span>
                {#if archiveSort.column === "modified"}
                  <ChevronDown class={`sort-chevron ${archiveSort.direction === "asc" ? "ascending" : ""}`} size={14} />
                {/if}
              </button>
            </span>
          </div>
          {#if visibleArchiveRows().length === 0}
            <div class="archive-empty">{archiveFilterActive() ? "没有匹配的项目" : "选择压缩包后显示文件列表"}</div>
          {:else}
            {#each visibleArchiveRows() as row}
              <div
                class="archive-table-row archive-row"
                class:selected={isArchiveRowHighlighted(row)}
                class:folder-row={row.kind === "folder"}
                class:archive-drop-target={row.kind === "folder" && (archiveInternalDropTargetPath === row.path || archiveExternalDropTargetPath === row.path)}
                data-archive-row-path={row.path}
                data-archive-row-depth={row.depth}
                data-archive-folder-path={row.kind === "folder" ? row.path : undefined}
                style={archiveTableColumnsStyle()}
                role="button"
                tabindex="0"
                oncontextmenu={(event) => openArchiveContextMenu(row, event)}
                onmousedown={(event) => startArchiveRowSelection(row.path, event)}
                onmouseenter={() => {
                  moveArchiveRowSelection(row.path);
                }}
                onclick={(event) => {
                  if (row.kind !== "folder") return;
                  const target = event.target as HTMLElement | null;
                  if (target?.closest("button, input, [data-row-action]")) return;
                  toggleArchiveFolder(row.path);
                }}
                ondblclick={() => {
                  if (row.kind !== "folder") {
                    previewArchiveRow(row);
                  }
                }}
                onkeydown={(event) => {
                  if (event.key === " " || event.key === "Enter") {
                    event.preventDefault();
                    if (row.kind === "folder") {
                      toggleArchiveFolder(row.path);
                    } else {
                      toggleArchivePath(row.path);
                    }
                  }
                }}
              >
                <span class="check-cell">
                  {#if row.kind === "folder"}
                    <input
                      type="checkbox"
                      aria-label={`选择 ${row.name}`}
                      checked={isArchiveRowSelected(row)}
                      data-partial={isArchiveRowPartiallySelected(row)}
                      onclick={(event) => event.stopPropagation()}
                      onchange={() => toggleArchivePath(row.path)}
                    />
                  {:else}
                    <input
                      type="checkbox"
                      aria-label={`选择 ${row.name}`}
                      checked={isArchivePathSelected(row.path)}
                      onclick={(event) => event.stopPropagation()}
                      onchange={() => toggleArchivePath(row.path)}
                    />
                  {/if}
                </span>
                <button
                  type="button"
                  class="file-name archive-file-cell"
	                  data-row-action
	                  style={`--tree-depth: ${row.depth}`}
	                  onmousedown={(event) => {
	                    event.stopPropagation();
	                    startArchiveFileDrag(row, event);
	                  }}
                  onmouseup={() => stopArchiveFileDrag()}
                  onclick={(event) => {
                    event.stopPropagation();
                    if (row.kind === "folder") {
                      toggleArchiveFolder(row.path);
                    } else if (!archiveFileDragStarted) {
                      toggleArchivePath(row.path);
                    }
                  }}
                  ondblclick={(event) => {
                    event.stopPropagation();
                    if (row.kind !== "folder") {
                      previewArchiveRow(row);
                    }
                  }}
                >
                  {#if row.kind === "folder"}
                    <span class="tree-toggle visual-toggle" aria-hidden="true">
                      {#if isArchiveFolderExpandedForDisplay(row.path)}
                        <ChevronDown size={15} strokeWidth={2} />
                      {:else}
                        <ChevronRight size={15} strokeWidth={2} />
                      {/if}
                    </span>
                    <span class="archive-file-drag-handle" title="拖到 Finder">
                      {#if systemIconSource(row.kind, row.name)}
                        <img class="system-file-icon" src={systemIconSource(row.kind, row.name) ?? ""} alt="" aria-hidden="true" draggable="false" />
                      {:else}
                        <span class="folder-icon"></span>
                      {/if}
                    </span>
                  {:else if row.kind === "image"}
                    <span class="archive-file-drag-handle" title="拖到 Finder">
                      <span class="tree-toggle spacer-toggle" aria-hidden="true"></span>
                      {#if systemIconSource(row.kind, row.name)}
                        <img class="system-file-icon" src={systemIconSource(row.kind, row.name) ?? ""} alt="" aria-hidden="true" draggable="false" />
                      {:else}
                        <span class="file-badge image"><FileImage size={20} strokeWidth={2} /></span>
                      {/if}
                    </span>
                  {:else}
                    <span class="archive-file-drag-handle" title="拖到 Finder">
                      <span class="tree-toggle spacer-toggle" aria-hidden="true"></span>
                      {#if systemIconSource(row.kind, row.name)}
                        <img class="system-file-icon" src={systemIconSource(row.kind, row.name) ?? ""} alt="" aria-hidden="true" draggable="false" />
                      {:else}
                        <span class="file-badge {row.kind}">{fileIconText(row.kind, row.name)}</span>
                      {/if}
                    </span>
                  {/if}
                  <span class="archive-name-copy">
                    <span>
                      {row.name}
                      {#if row.isEncrypted}
                        <small class="inline-badge">加密</small>
                      {/if}
                      {#if row.crc}
                        <small class="inline-badge muted">CRC</small>
                      {/if}
                    </span>
                    {#if archiveParentPath(row.path)}
                      <small>{archiveParentPath(row.path)}</small>
                    {/if}
                  </span>
                </button>
                <span>{row.type}</span>
                <span>{row.size}</span>
                <span>{row.modified}</span>
              </div>
            {/each}
          {/if}
        </div>
      </section>

      {#if archiveInfoPanelOpen}
        <div class="archive-info-backdrop" role="presentation" onclick={() => (archiveInfoPanelOpen = false)}>
          <div
            class="info-panel archive-info-drawer"
            aria-label="压缩包信息"
            role="dialog"
            aria-modal="true"
            tabindex="-1"
            onclick={(event) => event.stopPropagation()}
            onkeydown={(event) => {
              if (event.key === "Escape") archiveInfoPanelOpen = false;
            }}
          >
            <header class="archive-info-head">
              <h2>压缩包信息</h2>
              <button class="icon-button panel-close" type="button" aria-label="关闭压缩包信息" onclick={() => (archiveInfoPanelOpen = false)}>
                <X size={18} strokeWidth={2} />
              </button>
            </header>
          <dl>
            <div>
              <dt>格式</dt>
              <dd>{archiveInfo?.format ?? "-"}</dd>
            </div>
            <div>
              <dt>压缩大小</dt>
              <dd>{archiveInfo?.size_label ?? "-"}</dd>
            </div>
            <div>
              <dt>原始大小</dt>
              <dd>{archiveInfo?.properties.uncompressed_size_label ?? "-"}</dd>
            </div>
            <div>
              <dt>压缩率</dt>
              <dd>{archiveInfo?.properties.compression_ratio_label ?? "-"}</dd>
            </div>
            <div>
              <dt>文件数</dt>
              <dd>{archiveInfo?.file_count ?? 0}</dd>
            </div>
            <div>
              <dt>文件夹</dt>
              <dd>{archiveInfo?.folder_count ?? 0}</dd>
            </div>
            <div>
              <dt>创建时间</dt>
              <dd>{archiveInfo?.created_label ?? "-"}</dd>
            </div>
            <div>
              <dt>压缩方法</dt>
              <dd>{archiveInfo?.properties.method_summary ?? "-"}</dd>
            </div>
            <div>
              <dt>CRC</dt>
              <dd>{archiveInfo?.properties.crc_available ? "可用" : "未提供"}</dd>
            </div>
            <div>
              <dt>加密</dt>
              <dd>{archiveInfo?.properties.is_encrypted ? `是（${archiveInfo.properties.encrypted_count || "部分"}）` : "否"}</dd>
            </div>
            <div>
              <dt>分卷</dt>
              <dd>{archiveInfo?.properties.split.is_split ? `${archiveInfo.properties.split.volume_count} 卷` : "否"}</dd>
            </div>
          </dl>

          <div class="panel-action-grid">
            <button class="secondary-button" type="button" onclick={testCurrentArchiveIntegrity} disabled={!archiveInfo || busy}>测试</button>
            <button class="secondary-button" type="button" onclick={clearArchivePreviewCache} disabled={busy || previewCache.size === 0}>清缓存</button>
          </div>

          {#if archiveIntegrityMessage}
            <div class="compact-status success-text">{archiveIntegrityMessage}</div>
          {/if}

          <label class="compact-field">
            <span>冲突处理</span>
            <select bind:value={extractConflictStrategy} aria-label="解压冲突处理">
              <option value="overwrite">覆盖已有文件</option>
              <option value="skip">跳过已有文件</option>
              <option value="rename">自动重命名</option>
            </select>
          </label>

          {#if hasArchiveSafetyWarnings()}
            <div class="safety-note" class:danger={Boolean(archiveInfo?.safety.unsafe_paths)}>
              <strong>安全检查</strong>
              <span>{archiveSafetyText()}</span>
              {#if archiveInfo?.safety.samples.length}
                <small>{archiveInfo.safety.samples.join("、")}</small>
              {/if}
            </div>
          {/if}
          </div>
        </div>
      {/if}

      {#if archiveContextMenu}
        <button class="archive-context-dismiss" type="button" aria-label="关闭菜单" onclick={closeArchiveContextMenu}></button>
        <div
          class="archive-context-menu"
          role="menu"
          tabindex="-1"
          style={`left: ${archiveContextMenu.x}px; top: ${archiveContextMenu.y}px;`}
          oncontextmenu={(event) => event.preventDefault()}
        >
          {#if archiveContextMenu.row}
            <button role="menuitem" type="button" disabled={archiveContextMenu.row.kind === "folder" || busy} onclick={() => void previewContextArchiveRow()}>
              预览
            </button>
            <button
              role="menuitem"
              type="button"
              disabled={archiveContextMenu.row.kind === "folder" || busy || contextOpenWithLoading}
              onclick={() => void toggleContextOpenWithApps()}
            >
              打开方式
            </button>
            {#if contextOpenWithRowPath === archiveContextMenu.row.path}
              <div class="context-open-with" role="group" aria-label="打开方式">
                {#if contextOpenWithLoading}
                  <span class="context-status">正在读取打开方式...</span>
                {:else}
                  <button role="menuitem" type="button" disabled={busy} onclick={() => void openContextRowWithApplication(null)}>
                    默认应用
                  </button>
                  {#each contextOpenWithApps.slice(0, 5) as app}
                    <button role="menuitem" type="button" disabled={busy} title={app.path} onclick={() => void openContextRowWithApplication(app.path)}>
                      用 {app.name} 打开{app.isDefault ? "（默认）" : ""}
                    </button>
                  {/each}
                  <button role="menuitem" type="button" disabled={busy} onclick={() => void pickContextOpenWithApplication()}>
                    其他...
                  </button>
                {/if}
              </div>
            {/if}
            <button role="menuitem" type="button" disabled={busy} onclick={() => void renameContextArchiveRow()}>
              重命名
            </button>
            <button role="menuitem" class="danger" type="button" disabled={busy || contextArchiveRows().length === 0} onclick={() => void deleteContextArchiveRows()}>
              {contextDeleteLabel()}
            </button>
            <span class="context-separator"></span>
          {/if}
          <button role="menuitem" type="button" disabled={!archiveInfo || busy} onclick={() => void createArchiveFolder()}>
            新建文件夹
          </button>
	          <button role="menuitem" type="button" disabled={!archiveInfo || busy} onclick={() => void addItemsToArchive()}>
	            添加文件或文件夹
	          </button>
          <button role="menuitem" type="button" disabled={!archiveInfo || busy} onclick={() => void saveArchiveAs()}>
            另存为压缩包
          </button>
        </div>
      {/if}

    {:else}
      <header class="titlebar task-titlebar" data-tauri-drag-region onmousedown={startWindowDrag} role="presentation">
        <div class="task-title">
          <strong>压缩</strong>
        </div>
        <button class="primary-button titlebar-action with-icon" type="button" onclick={compressSelected} disabled={allCompressRows().length === 0 || busy} data-no-drag>
          <Archive size={18} strokeWidth={1.9} />
          <span>{busy ? "处理中..." : "开始压缩"}</span>
        </button>
      </header>

      <section class="compress-body">
        <div class="compress-left">
          <div class="selected-summary">
            <span>已添加 {allCompressRows().length} 个项目</span>
            <button class="icon-button clear-button" type="button" onclick={() => void removeSelectedCompressItems()} disabled={selectedCompressPaths.length === 0} aria-label="移除所选" title="移除所选">
              <Trash2 size={18} strokeWidth={1.9} />
            </button>
          </div>
          <div
            class="selected-list compress-tree"
            role="tree"
            aria-label="待压缩项目"
            tabindex="0"
            onmouseup={stopCompressRowSelection}
            onmouseleave={stopCompressRowSelection}
          >
            {#if allCompressRows().length === 0}
              <div class="empty-row">暂无已选项目</div>
            {:else}
              <div class="compress-tree-head">
                <span class="check-cell">
                  <input
                    type="checkbox"
                    aria-label="全选待压缩项目"
                    checked={allCompressRows().length > 0 && selectedCompressPaths.length === allCompressRows().length}
                    onchange={toggleAllCompressRows}
                  />
                </span>
                <span>名称</span>
                <span>大小</span>
              </div>
              {#each flattenCompressTree() as item}
                <div
                  class="compress-tree-row"
                  class:selected={isCompressPathSelected(item.path)}
                  role="treeitem"
                  aria-level={item.depth + 1}
                  aria-selected={isCompressPathSelected(item.path)}
                  tabindex="-1"
                  onmousedown={(event) => startCompressRowSelection(item.path, event)}
                  onmouseenter={() => moveCompressRowSelection(item.path)}
                >
                  <span class="check-cell">
                    <input
                      type="checkbox"
                      aria-label={`选择 ${item.name}`}
                      checked={isCompressPathSelected(item.path)}
                      onchange={() => toggleCompressPath(item.path)}
                    />
                  </span>
                  <button
                    class="file-name compress-file-cell"
                    type="button"
                    style={`--tree-depth: ${item.depth}`}
                    onclick={() => {
                      if (item.kind === "folder") {
                        toggleCompressFolder(item.path);
                      } else {
                        toggleCompressPath(item.path);
                      }
                    }}
                  >
                    {#if item.kind === "folder"}
                      <span class="tree-toggle visual-toggle">
                        {#if isCompressFolderExpanded(item.path)}
                          <ChevronDown size={15} strokeWidth={2} />
                        {:else}
                          <ChevronRight size={15} strokeWidth={2} />
                        {/if}
                      </span>
                      {#if systemIconSource(item.kind, item.name, item.path)}
                        <img class="system-file-icon" src={systemIconSource(item.kind, item.name, item.path) ?? ""} alt="" aria-hidden="true" draggable="false" />
                      {:else}
                        <span class="folder-icon"></span>
                      {/if}
                    {:else if item.kind === "image"}
                      <span class="tree-toggle spacer-toggle"></span>
                      {#if systemIconSource(item.kind, item.name, item.path)}
                        <img class="system-file-icon" src={systemIconSource(item.kind, item.name, item.path) ?? ""} alt="" aria-hidden="true" draggable="false" />
                      {:else}
                        <span class="file-badge image"><FileImage size={20} strokeWidth={2} /></span>
                      {/if}
                    {:else}
                      <span class="tree-toggle spacer-toggle"></span>
                      {#if systemIconSource(item.kind, item.name, item.path)}
                        <img class="system-file-icon" src={systemIconSource(item.kind, item.name, item.path) ?? ""} alt="" aria-hidden="true" draggable="false" />
                      {:else}
                        <span class="file-badge {item.kind}">{fileIconText(item.kind, item.name)}</span>
                      {/if}
                    {/if}
                    <span class="compress-name-copy">{item.name}</span>
                  </button>
                  <span class="compress-size-cell">{item.size}</span>
                </div>
              {/each}
            {/if}
          </div>

          <div class="total-size">
            <Folder size={20} strokeWidth={1.8} />
            <span>总大小：{formatSize(totalSelectedSize())}</span>
          </div>
        </div>

        <aside class="settings-panel">
          <button class="drop-zone" type="button" onclick={() => void pickCompressSources()} aria-label="添加文件或文件夹">
            <span class="folder-add">
              <Folder size={48} strokeWidth={1.3} />
              <PackagePlus size={24} strokeWidth={1.8} />
            </span>
            <strong>添加文件或文件夹</strong>
            <span class="drop-hint">点击选择，或拖拽到此处</span>
          </button>

          <label class="field-block">
            <span>压缩包名称</span>
            <span class="input-group">
              <input bind:value={archiveName} placeholder="输入压缩包名称" aria-label="压缩包名称" />
              <select class="extension-select" bind:value={archiveFormat} aria-label="压缩格式">
                {#each supportedCompressFormats as format}
                  <option value={format}>.{compressExtension(format)}</option>
                {/each}
              </select>
            </span>
          </label>

          <label class="field-block">
            <span>保存位置</span>
            <span class="path-picker">
              <input bind:value={saveDir} placeholder="选择保存位置" aria-label="保存位置" />
              <button class="secondary-button browse-button" type="button" onclick={chooseCompressSaveDir}>浏览...</button>
            </span>
          </label>

          <div class="field-block">
            <span>Mac 文件</span>
            <label class="switch-row">
              <input
                type="checkbox"
                checked={skipDsStore}
                disabled={busy}
                onchange={(event) => void setSkipDsStore(event.currentTarget.checked)}
              />
              <span>不压缩 .DS_Store</span>
            </label>
            <label class="switch-row">
              <input
                type="checkbox"
                checked={skipMacosMetadata}
                disabled={busy}
                onchange={(event) => void setSkipMacosMetadata(event.currentTarget.checked)}
              />
              <span>不压缩 macOS 元数据</span>
            </label>
          </div>

          <div class="field-block">
            <span>任务</span>
            <label class="switch-row">
              <input type="checkbox" bind:checked={batchCompressQueue} disabled={selectedItems.length <= 1 || busy} />
              <span>分别压缩顶层项目</span>
            </label>
            <label class="switch-row">
              <input type="checkbox" bind:checked={testAfterCompress} disabled={busy} />
              <span>压缩后测试完整性</span>
            </label>
          </div>

          {#if compressionLevelConfig(archiveFormat)}
            {@const levelConfig = compressionLevelConfig(archiveFormat)}
            <div class="field-block">
              <span>压缩级别</span>
              <div class="range-control">
                <div class="range-head">
                  <span>{levelConfig.low}</span>
                  <strong>{compressionLevel}</strong>
                  <span>{levelConfig.high}</span>
                </div>
                <input
                  type="range"
                  min={levelConfig.min}
                  max={levelConfig.max}
                  value={compressionLevel}
                  oninput={(event) => compressionLevel = Number(event.currentTarget.value)}
                  aria-label="压缩级别"
                />
              </div>
            </div>
          {/if}

          {#if supportsCompressionPassword(archiveFormat)}
            <label class="field-block">
              <span>压缩密码</span>
              <input class="plain-input" type="password" bind:value={compressionPassword} placeholder="可选" aria-label="压缩密码" />
            </label>
          {/if}

          {#if archiveFormat === "7Z"}
            <div class="field-block">
              <span>7Z 参数</span>
              <div class="compact-field-grid">
                <label class="mini-field">
                  <span>方法</span>
                  <select bind:value={sevenZipMethod} aria-label="7Z 压缩方法">
                    <option value="LZMA2">LZMA2</option>
                    <option value="LZMA">LZMA</option>
                  </select>
                </label>
                <label class="mini-field">
                  <span>字典 MB</span>
                  <input
                    type="number"
                    min="1"
                    max="1024"
                    step="1"
                    value={sevenZipDictionarySizeMb}
                    oninput={(event) => sevenZipDictionarySizeMb = Math.min(1024, Math.max(1, Number(event.currentTarget.value) || 1))}
                    aria-label="7Z 字典大小"
                  />
                </label>
              </div>
              <div class="range-control">
                <div class="range-head">
                  <span>1 线程</span>
                  <strong>{compressThreads}</strong>
                  <span>32 线程</span>
                </div>
                <input
                  type="range"
                  min="1"
                  max="32"
                  value={compressThreads}
                  oninput={(event) => compressThreads = Number(event.currentTarget.value)}
                  aria-label="7Z 线程数"
                />
              </div>
              <label class="switch-row">
                <input type="checkbox" bind:checked={sevenZipSolid} />
                <span>固实压缩</span>
              </label>
            </div>
          {/if}

          {#if supportsSplitArchive(archiveFormat)}
            <div class="field-block">
              <span>分卷压缩</span>
              <label class="switch-row">
                <input type="checkbox" bind:checked={splitArchive} />
                <span>启用</span>
              </label>
              {#if splitArchive}
                <label class="split-size">
                  <input
                    type="number"
                    min="1"
                    step="1"
                    value={splitSizeMb}
                    oninput={(event) => splitSizeMb = Math.max(1, Number(event.currentTarget.value) || 1)}
                    aria-label="分卷大小"
                  />
                  <span>MB / 卷</span>
                </label>
              {/if}
            </div>
          {/if}

          <div class="status-area">
            {#if errorMessage}
              <span class="error-text">{errorMessage}</span>
            {:else if statusMessage}
              <span class="success-text">{statusMessage}</span>
            {:else}
              <span>压缩结果会保存到所选位置</span>
            {/if}
          </div>

        </aside>
      </section>
    {/if}
  </section>

  {#if activeAppPanel}
    <div class="app-panel-backdrop" role="presentation" onclick={closeAppPanel}>
      <div
        class="app-panel-window"
        role="dialog"
        aria-modal="true"
        aria-labelledby="app-panel-title"
        tabindex="-1"
        onclick={(event) => event.stopPropagation()}
        onkeydown={(event) => {
          if (event.key === "Escape") closeAppPanel();
        }}
      >
        <header class="app-panel-head">
          <div>
            <h2 id="app-panel-title">{activeAppPanel === "recent" ? "最近文件" : "设置"}</h2>
            <p>{activeAppPanel === "recent" ? `${recentFiles.length} 个项目` : "通用偏好"}</p>
          </div>
          <button class="icon-button panel-close" type="button" aria-label="关闭" onclick={closeAppPanel}>
            <X size={19} strokeWidth={1.9} />
          </button>
        </header>

        {#if activeAppPanel === "recent"}
          <div class="recent-window-content">
            <form
              class="recent-search"
              role="search"
              onsubmit={(event) => {
                event.preventDefault();
                void searchRecentFiles();
              }}
            >
              <label class="recent-search-field" aria-label="搜索最近文件">
                <Search size={17} strokeWidth={2} />
                <input
                  type="search"
                  bind:this={recentSearchInput}
                  bind:value={homeSearch}
                  placeholder="搜索最近文件"
                />
              </label>
              <button class="recent-search-button" type="submit">搜索</button>
              {#if homeSearchMessage}
                <span class="recent-search-message">{homeSearchMessage}</span>
              {/if}
            </form>

            <div class="recent-window-list">
              {#if recentFiles.length === 0}
                <div class="empty-row">暂无最近文件</div>
              {:else if filteredRecentFiles().length === 0}
                <div class="empty-row">没有匹配的最近文件</div>
              {:else}
                {#each filteredRecentFiles() as file}
                  <div class="recent-window-row">
                    <button type="button" class="recent-open-button" onclick={() => void openRecentFile(file.path)}>
                      <span class="file-name">
                        {#if systemIconSource(file.kind, file.name, file.path)}
                          <img class="system-file-icon" src={systemIconSource(file.kind, file.name, file.path) ?? ""} alt="" aria-hidden="true" draggable="false" />
                        {:else}
                          <span class="file-badge {file.kind}">{fileIconText(file.kind, file.name)}</span>
                        {/if}
                        <span>
                          <strong>{file.name}</strong>
                          <small>{file.path}</small>
                        </span>
                      </span>
                      <span>{file.format}</span>
                      <span>{file.time}</span>
                    </button>
                    <button class="icon-button" type="button" aria-label={`显示 ${file.name} 的位置`} onclick={() => revealItemInDir(file.path)}>
                      <FolderOpen size={17} strokeWidth={1.9} />
                    </button>
                  </div>
                {/each}
              {/if}
            </div>
          </div>

          <footer class="app-panel-actions">
            <button class="secondary-button" type="button" disabled={recentFiles.length === 0} onclick={clearRecentFiles}>清空最近</button>
          </footer>
        {:else}
          <div class="settings-window-content">
            <div class="field-block">
              <span class="label-with-help">默认压缩格式</span>
              <select class="format-select" bind:value={defaultArchiveFormat} aria-label="默认压缩格式">
                {#each supportedCompressFormats as format}
                  <option value={format}>{format}（.{compressExtension(format)}）</option>
                {/each}
              </select>
            </div>

            <div class="field-block">
              <label class="switch-row">
                <input type="checkbox" bind:checked={defaultSaveDirEnabled} />
                <span>启用默认保存位置</span>
              </label>
              <span class="path-picker">
                <input
                  bind:value={defaultSaveDir}
                  disabled={!defaultSaveDirEnabled}
                  placeholder={defaultSaveDirEnabled ? "选择保存位置" : "未启用，默认保存到所选文件夹根目录"}
                  aria-label="默认保存位置"
                />
                <button class="secondary-button browse-button" type="button" disabled={!defaultSaveDirEnabled} onclick={chooseDefaultSaveDir}>浏览...</button>
              </span>
            </div>

            <div class="field-block">
              <label class="switch-row">
                <input type="checkbox" bind:checked={defaultExtractDirEnabled} />
                <span>启用默认解压位置</span>
              </label>
              <span class="path-picker">
                <input
                  bind:value={defaultExtractDir}
                  disabled={!defaultExtractDirEnabled}
                  placeholder={defaultExtractDirEnabled ? "选择解压位置" : "未启用，默认解压到压缩包位置"}
                  aria-label="默认解压位置"
                />
                <button class="secondary-button browse-button" type="button" disabled={!defaultExtractDirEnabled} onclick={chooseDefaultExtractDir}>浏览...</button>
              </span>
            </div>

            <div class="settings-format-note">
              <span>解压支持</span>
              <strong>{supportedExtractFormats}</strong>
            </div>

            <div class="settings-format-note permission-settings-note">
              <span>权限与访问</span>
              <strong>可自动把常见压缩包默认打开方式设为 Packo；完全磁盘访问仅在处理受保护位置时需要。</strong>
              <button class="secondary-button" type="button" onclick={openPermissionGuide}>查看引导</button>
            </div>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  {#if permissionGuideOpen}
    {@const guideSlide = currentPermissionGuideSlide()}
    {@const isLastGuideSlide = permissionGuideStep === permissionGuideSlides.length - 1}
    <div class="permission-guide-backdrop" role="presentation">
      <div
        class="permission-guide-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="permission-guide-title"
        tabindex="-1"
        onclick={(event) => event.stopPropagation()}
        onkeydown={(event) => {
          if (event.key === "Escape") closePermissionGuide();
        }}
      >
        <header class="permission-guide-head">
          <span class="permission-guide-icon">
            <ShieldCheck size={30} strokeWidth={1.8} />
          </span>
          <div>
            <h2 id="permission-guide-title">{guideSlide.title}</h2>
            <p>{guideSlide.description}</p>
          </div>
          <button class="icon-button panel-close" type="button" aria-label="关闭新手引导" onclick={closePermissionGuide}>
            <X size={19} strokeWidth={1.9} />
          </button>
        </header>

        <div class="permission-steps onboarding-steps" class:last-guide-step={isLastGuideSlide}>
          <div
            class="onboarding-image-frame"
            class:missing={guideImageMissing(guideSlide.image)}
            role="img"
            aria-label={guideSlide.imageAlt}
            style={`--guide-image: url("${guideSlide.image}")`}
          >
            {#if guideImageMissing(guideSlide.image)}
              <div class="onboarding-image-placeholder">
                <Info size={28} strokeWidth={1.9} />
                <strong>{guideSlide.image.split("/").pop()}</strong>
                <span>把这张引导图放到 static{guideSlide.image}</span>
              </div>
            {:else}
              <img
                class="onboarding-image-probe"
                src={guideSlide.image}
                alt=""
                aria-hidden="true"
                onerror={() => markGuideImageMissing(guideSlide.image)}
              />
            {/if}
          </div>

          {#if isLastGuideSlide}
            <article class="permission-step onboarding-permission-step">
              <span class="permission-step-icon">
                <Archive size={21} strokeWidth={1.9} />
              </span>
              <div>
                <strong>默认打开方式改为 Packo</strong>
                <p>写入 ZIP、7Z、RAR、TAR、GZ、BZ2、XZ、ZSTD、LZMA、LZ4、ISO 等格式的系统默认打开方式。</p>
              </div>
              <button class="secondary-button with-icon" type="button" disabled={defaultOpenerSetting} onclick={() => void setPackoAsDefaultArchiveOpener()}>
                {#if defaultOpenerSetting}
                  <LoaderCircle size={17} strokeWidth={1.9} class="spinning-icon" />
                {:else}
                  <Check size={17} strokeWidth={2} />
                {/if}
                <span>{defaultOpenerSetting ? "正在设置" : "设为默认"}</span>
              </button>
            </article>
          {/if}
        </div>

        {#if permissionGuideMessage}
          <p class="permission-guide-message">{permissionGuideMessage}</p>
        {/if}

        <footer class="permission-guide-actions">
          <div class="onboarding-dots" aria-label="引导页进度">
            {#each permissionGuideSlides as _, index}
              <button
                type="button"
                class:active={index === permissionGuideStep}
                aria-label={`第 ${index + 1} 页`}
                onclick={() => {
                  permissionGuideMessage = "";
                  permissionGuideStep = index;
                }}
              ></button>
            {/each}
          </div>
          <button class="secondary-button" type="button" onclick={closePermissionGuide}>跳过</button>
          <button class="secondary-button" type="button" disabled={permissionGuideStep === 0} onclick={previousPermissionGuideStep}>上一步</button>
          <button class="primary-button with-icon" type="button" onclick={nextPermissionGuideStep}>
            {#if isLastGuideSlide}
              <Check size={18} strokeWidth={2} />
            {:else}
              <ChevronRight size={18} strokeWidth={2} />
            {/if}
            <span>{guideSlide.primaryLabel || (isLastGuideSlide ? "开始使用" : "下一步")}</span>
          </button>
        </footer>
      </div>
    </div>
  {/if}

  {#if destinationPrompt}
    <div class="progress-backdrop password-backdrop" role="presentation">
      <div
        class="progress-dialog destination-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="destination-dialog-title"
      >
        <form
          class="password-form destination-form"
          onsubmit={(event) => {
            event.preventDefault();
            submitDestinationPrompt();
          }}
        >
          <header class="progress-dialog-head">
            <span class="progress-icon password-icon">
              <FolderOpen size={30} strokeWidth={1.8} />
            </span>
            <div>
              <h2 id="destination-dialog-title">{destinationPrompt.title}</h2>
              <p>{destinationPrompt.description}</p>
            </div>
          </header>

          <div class="destination-options">
            <div class="destination-option" class:active={destinationPromptMode === "current"}>
              <label class="destination-radio">
                <input
                  type="radio"
                  name="destination-mode"
                  checked={destinationPromptMode === "current"}
                  onchange={() => (destinationPromptMode = "current")}
                />
                <span>保存到当前文件路径下</span>
              </label>
              <input class="destination-path-input" type="text" value={destinationPrompt.currentPath} readonly aria-label="当前文件路径" />
            </div>

            <div class="destination-option" class:active={destinationPromptMode === "custom"}>
              <label class="destination-radio">
                <input
                  type="radio"
                  name="destination-mode"
                  checked={destinationPromptMode === "custom"}
                  onchange={() => (destinationPromptMode = "custom")}
                />
                <span>选择其他文件夹保存</span>
              </label>
              <span class="destination-path-picker">
                <input
                  class="destination-path-input"
                  type="text"
                  bind:value={destinationPromptCustomPath}
                  aria-label="其他保存位置"
                  onfocus={() => (destinationPromptMode = "custom")}
                />
                <button class="secondary-button browse-button" type="button" onclick={browseDestinationPrompt}>浏览...</button>
              </span>
            </div>
          </div>

          {#if destinationPromptMessage}
            <p class="password-message">{destinationPromptMessage}</p>
          {/if}

          <footer class="progress-actions">
            <button class="secondary-button with-icon" type="button" onclick={cancelDestinationPrompt}>
              <X size={19} strokeWidth={1.9} />
              <span>取消</span>
            </button>
            <button class="primary-button with-icon" type="submit">
              <Check size={19} strokeWidth={1.9} />
              <span>继续</span>
            </button>
          </footer>
        </form>
      </div>
    </div>
  {/if}

  {#if archiveTextPrompt}
    <div class="progress-backdrop password-backdrop" role="presentation">
      <div
        class="progress-dialog password-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="archive-text-dialog-title"
      >
        <form
          class="password-form"
          onsubmit={(event) => {
            event.preventDefault();
            void submitArchiveTextPrompt();
          }}
        >
          <header class="progress-dialog-head">
            <span class="progress-icon password-icon">
              <Folder size={30} strokeWidth={1.8} />
            </span>
            <div>
              <h2 id="archive-text-dialog-title">{archiveTextPromptTitle(archiveTextPrompt)}</h2>
              <p>{archiveTextPromptDescription(archiveTextPrompt)}</p>
            </div>
          </header>

          <label class="password-field">
            <span>{archiveTextPromptLabel(archiveTextPrompt)}</span>
            <input type="text" bind:this={archiveTextPromptInput} bind:value={archiveTextPromptValue} aria-label={archiveTextPromptLabel(archiveTextPrompt)} />
          </label>

          {#if archiveTextPromptMessage}
            <p class="password-message">{archiveTextPromptMessage}</p>
          {/if}

          <footer class="progress-actions">
            <button class="secondary-button with-icon" type="button" onclick={cancelArchiveTextPrompt}>
              <X size={19} strokeWidth={1.9} />
              <span>取消</span>
            </button>
            <button class="primary-button with-icon" type="submit" disabled={busy}>
              <Check size={19} strokeWidth={1.9} />
              <span>确定</span>
            </button>
          </footer>
        </form>
      </div>
    </div>
  {/if}

  {#if passwordPromptAction}
    <div class="progress-backdrop password-backdrop" role="presentation">
      <div
        class="progress-dialog password-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="password-dialog-title"
      >
        <form
          class="password-form"
          onsubmit={(event) => {
            event.preventDefault();
            void submitArchivePasswordPrompt();
          }}
        >
          <header class="progress-dialog-head">
            <span class="progress-icon password-icon">
              <KeyRound size={30} strokeWidth={1.8} />
            </span>
            <div>
              <h2 id="password-dialog-title">输入解压密码</h2>
              <p>{passwordPromptTarget(passwordPromptAction)}</p>
            </div>
          </header>

          <label class="password-field">
            <span>解压密码</span>
            <input type="password" bind:value={passwordPromptValue} aria-label="解压密码" />
          </label>

          {#if passwordPromptMessage}
            <p class="password-message">{passwordPromptMessage}</p>
          {/if}

          <footer class="progress-actions">
            <button class="secondary-button with-icon" type="button" onclick={cancelArchivePasswordPrompt}>
              <X size={19} strokeWidth={1.9} />
              <span>取消</span>
            </button>
            <button class="primary-button with-icon" type="submit">
              <Check size={19} strokeWidth={1.9} />
              <span>继续</span>
            </button>
          </footer>
        </form>
      </div>
    </div>
  {/if}

  {#if archivePromiseDragProgress}
    <aside class="promise-drag-progress" class:failed={archivePromiseDragProgress.status === "failed"} class:done={archivePromiseDragProgress.status === "completed" && archivePromiseDragProgress.completed >= archivePromiseDragProgress.total} aria-live="polite">
      <span class="promise-drag-icon">
        {#if archivePromiseDragProgress.status === "failed"}
          <CircleX size={20} strokeWidth={1.9} />
        {:else if archivePromiseDragProgress.status === "completed" && archivePromiseDragProgress.completed >= archivePromiseDragProgress.total}
          <CircleCheck size={20} strokeWidth={1.9} />
        {:else}
          <LoaderCircle size={20} strokeWidth={1.9} />
        {/if}
      </span>
      <div class="promise-drag-body">
        <div class="promise-drag-head">
          <strong>{archivePromiseDragTitle()}</strong>
          <span>{archivePromiseDragProgress.completed} / {archivePromiseDragProgress.total}</span>
        </div>
        <p>{archivePromiseDragProgress.error || archivePromiseDragProgress.message || archivePromiseDragProgress.currentItem}</p>
        <div class="promise-drag-meter" aria-label="拖出进度">
          <span style={`width: ${archivePromiseDragPercent()}%`}></span>
        </div>
      </div>
    </aside>
  {/if}

  {#if extractTask}
    <div class="progress-backdrop" role="presentation">
      <div class="progress-dialog" role="dialog" aria-modal="true" aria-labelledby="extract-progress-title">
        <header class="progress-dialog-head">
          <span class="progress-icon" class:done={extractTask.status === "completed"} class:failed={extractTask.status === "failed" || extractTask.status === "canceled"}>
            {#if extractTask.status === "completed"}
              <CircleCheck size={30} strokeWidth={1.8} />
            {:else if extractTask.status === "failed" || extractTask.status === "canceled"}
              <CircleX size={30} strokeWidth={1.8} />
            {:else}
              <LoaderCircle size={30} strokeWidth={1.8} />
            {/if}
          </span>
          <div>
            <h2 id="extract-progress-title">{extractStatusLabel(extractTask.status)}</h2>
            <p>{extractTaskArchiveName || archiveInfo?.name || "压缩包"}</p>
          </div>
        </header>

        <div class="progress-group">
          <div class="progress-line">
            <div class="progress-line-head">
              <span>{extractTaskMode === "edit" ? "当前阶段" : extractTaskMode === "compress" ? "当前项目" : "当前文件"}</span>
              <strong>{currentFileProgressPercent()}%</strong>
            </div>
            <div class="progress-meter" aria-label={extractTaskMode === "edit" ? "当前阶段进度" : extractTaskMode === "compress" ? "当前项目进度" : "当前文件进度"}>
              <span class="progress-fill current" style={`width: ${currentFileProgressPercent()}%`}></span>
            </div>
            <div class="progress-line-foot">
              <span>{extractTask.current_item || (extractTask.status === "completed" ? "全部完成" : "准备中")}</span>
              <span>{formatSize(extractTask.current_bytes)} / {formatSize(extractTask.current_total_bytes || 1)}</span>
            </div>
          </div>

          <div class="progress-line">
            <div class="progress-line-head">
              <span>总体进度</span>
              <strong>{extractProgressPercent()}%</strong>
            </div>
            <div class="progress-meter" aria-label="总体进度">
              <span class="progress-fill total" style={`width: ${extractProgressPercent()}%`}></span>
            </div>
            <div class="progress-line-foot">
              <span>{extractTask.completed} / {extractTask.total} 个项目</span>
              <span>{formatSize(extractTask.completed_bytes)} / {formatSize(extractTask.total_bytes || 1)}</span>
            </div>
          </div>
        </div>

        <dl class="progress-details">
          <div>
            <dt>状态</dt>
            <dd>{extractStatusLabel(extractTask.status)}</dd>
          </div>
          <div>
            <dt>{extractTaskMode === "compress" || extractTaskMode === "edit" ? "输出文件" : "保存位置"}</dt>
            <dd>{extractTask.output_path || (extractTaskMode === "preview" ? "临时预览目录" : extractTaskMode === "compress" ? "-" : extractDir) || "-"}</dd>
          </div>
          {#if extractTaskSelectionCount > 0}
            <div>
              <dt>范围</dt>
              <dd>已选择 {extractTaskSelectionCount} 个项目</dd>
            </div>
          {/if}
        </dl>

        <p class:progress-error={extractTask.status === "failed"} class:progress-success={extractTask.status === "completed"} class="progress-message">
          {extractTask.error || extractTask.message}
        </p>

        <footer class="progress-actions">
          {#if extractTask.status === "running"}
            {#if extractTaskMode === "edit"}
              <button class="secondary-button with-icon" type="button" disabled>
                <LoaderCircle size={19} strokeWidth={1.9} />
                <span>保存中</span>
              </button>
            {:else}
              <button class="secondary-button with-icon" type="button" onclick={pauseExtractTask}>
                <Pause size={19} strokeWidth={1.9} />
                <span>暂停</span>
              </button>
              <button class="secondary-button with-icon danger" type="button" onclick={cancelExtractTask}>
                <X size={19} strokeWidth={1.9} />
                <span>取消</span>
              </button>
            {/if}
          {:else if extractTask.status === "paused"}
            <button class="primary-button with-icon" type="button" onclick={resumeExtractTask}>
              <Play size={19} strokeWidth={1.9} />
              <span>继续</span>
            </button>
            <button class="secondary-button with-icon danger" type="button" onclick={cancelExtractTask}>
              <X size={19} strokeWidth={1.9} />
              <span>取消</span>
            </button>
          {:else if extractTask.status === "canceling"}
            <button class="secondary-button with-icon" type="button" disabled>
              <LoaderCircle size={19} strokeWidth={1.9} />
              <span>取消中</span>
            </button>
          {:else}
            {#if extractTask.status === "completed"}
              <button class="secondary-button with-icon" type="button" onclick={openExtractOutput}>
                {#if extractTaskMode === "preview"}
                  <ExternalLink size={19} strokeWidth={1.9} />
                  <span>打开文件</span>
                {:else}
                  <FolderOpen size={19} strokeWidth={1.9} />
                  <span>在 Finder 中显示</span>
                {/if}
              </button>
            {/if}
            <button class="primary-button with-icon" type="button" onclick={closeExtractProgress}>
              <Check size={19} strokeWidth={1.9} />
              <span>完成</span>
            </button>
          {/if}
        </footer>
      </div>
    </div>
  {/if}
</main>

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(html),
  :global(body) {
    width: 100%;
    min-width: 0;
    min-height: 100%;
    margin: 0;
    background: transparent;
  }

  :global(body) {
    overflow: hidden;
    font-family:
      Inter,
      "PingFang SC",
      "Microsoft YaHei",
      "Segoe UI",
      sans-serif;
    color: #1f1f1f;
    -webkit-font-smoothing: antialiased;
    text-rendering: optimizeLegibility;
    user-select: none;
    -webkit-user-select: none;
  }

  button,
  input {
    font: inherit;
  }

  button {
    cursor: pointer;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.58;
  }

  input {
    min-width: 0;
    user-select: text;
    -webkit-user-select: text;
  }

  :global(*) {
    scrollbar-width: thin;
    scrollbar-color: #d8cbb9 transparent;
  }

  :global(*::-webkit-scrollbar) {
    width: 10px;
    height: 10px;
  }

  :global(*::-webkit-scrollbar-track) {
    background: transparent;
  }

  :global(*::-webkit-scrollbar-thumb) {
    min-height: 48px;
    border: 3px solid transparent;
    border-radius: 999px;
    background: #d8cbb9;
    background-clip: content-box;
  }

  :global(*::-webkit-scrollbar-thumb:hover) {
    background: #c4b29d;
    background-clip: content-box;
  }

  .stage {
    min-height: 100vh;
    padding: 0;
    display: grid;
    place-items: center;
    background: #fbf8f1;
  }

  .stage.compact {
    padding: 0;
  }

  .app-shell {
    width: 100%;
    height: 100vh;
    overflow: hidden;
    border: 1px solid rgba(120, 100, 70, 0.12);
    border-radius: 0;
    background: #fbf8f1;
    box-shadow: none;
    display: flex;
    flex-direction: column;
  }

  .task-shell {
    width: 100%;
    height: 100vh;
    border-radius: 0;
    background: #fffdf9;
  }

  .workspace-loading {
    flex: 1 1 auto;
    min-height: 0;
    padding: 42px;
    display: grid;
    place-items: center;
    text-align: center;
    color: #6f675d;
  }

  .workspace-loading > div {
    display: grid;
    gap: 8px;
    justify-items: center;
  }

  .workspace-loading strong {
    color: #1f1f1f;
    font-size: 18px;
    font-weight: 700;
  }

  .workspace-loading > div span {
    max-width: min(420px, calc(100vw - 64px));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .workspace-loading-icon {
    width: 58px;
    height: 58px;
    margin-bottom: 16px;
    border-radius: 16px;
    background: #fff6dd;
    color: #d79500;
    display: grid;
    place-items: center;
    box-shadow: inset 0 0 0 1px rgba(240, 180, 35, 0.22);
  }

  .workspace-loading-icon :global(svg) {
    animation: progress-spin 1.1s linear infinite;
  }

  .titlebar {
    height: 64px;
    flex: 0 0 auto;
    border-bottom: 1px solid #ede4d5;
    background: rgba(255, 253, 249, 0.88);
    display: grid;
    align-items: center;
    position: relative;
    user-select: none;
    -webkit-app-region: drag;
  }

  .titlebar button,
  .titlebar [data-no-drag] {
    -webkit-app-region: no-drag;
  }

  .main-titlebar {
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 16px;
    padding: 0 24px 0 84px;
  }

  .task-titlebar {
    height: 64px;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 16px;
    padding: 0 28px 0 98px;
  }

  .task-title {
    width: fit-content;
    border: 0;
    background: transparent;
    color: #1f1f1f;
    font-size: 17px;
    font-weight: 650;
    display: inline-flex;
    align-items: center;
    gap: 14px;
    padding: 0;
  }

  .titlebar-action {
    justify-self: end;
    height: 38px;
    min-width: 112px;
    padding: 0 16px;
    border-radius: 10px;
    font-size: 14px;
    white-space: nowrap;
  }

  .app-tabs {
    justify-self: end;
    min-width: 0;
    padding: 4px;
    border: 1px solid #e7ded1;
    border-radius: 12px;
    background: #fffaf1;
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .app-tabs button {
    height: 34px;
    min-width: 82px;
    border: 0;
    border-radius: 9px;
    background: transparent;
    color: #5f574d;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    padding: 0 12px;
    font-size: 14px;
    font-weight: 600;
  }

  .app-tabs button:hover,
  .app-tabs button.active {
    background: #fff0c8;
    color: #1f1f1f;
  }

  .home-layout {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr;
  }

  .home-content {
    min-width: 0;
    overflow: auto;
    padding: 32px;
    display: grid;
    place-items: center;
  }

  .home-center {
    display: grid;
    justify-items: center;
    gap: 28px;
  }

  .home-center h1 {
    margin: 0;
    color: #1f1f1f;
    font-size: 28px;
    font-weight: 760;
  }

  .hero-actions {
    width: min(100%, 468px);
    display: grid;
    grid-template-columns: repeat(2, minmax(180px, 220px));
    justify-content: center;
    gap: 28px;
  }

  .action-card {
    width: 100%;
    aspect-ratio: 1;
    min-height: 0;
    padding: 26px;
    border: 1px solid #efe5d6;
    border-radius: 16px;
    background: #fff;
    box-shadow: 0 8px 24px rgba(80, 60, 30, 0.06);
    display: grid;
    grid-template-columns: 1fr;
    align-items: center;
    justify-items: center;
    align-content: center;
    gap: 18px;
    text-align: center;
    color: inherit;
    font: inherit;
    cursor: pointer;
    transition:
      transform 0.18s ease,
      box-shadow 0.18s ease;
  }

  .action-card:hover {
    transform: translateY(-1px);
    box-shadow: 0 10px 28px rgba(80, 60, 30, 0.08);
  }

  .home-action-icon {
    width: 86px;
    height: 86px;
    border: 1px solid #e8dfd2;
    border-radius: 18px;
    background: #fffaf1;
    color: #3d3a35;
    display: grid;
    place-items: center;
  }

  .compress-icon {
    background: #fff7e1;
    color: #3a3328;
  }

  .extract-icon {
    background: #eef7f3;
    color: #2f4d44;
  }

  .action-copy {
    min-width: 0;
    display: grid;
    justify-items: center;
  }

  .action-copy strong {
    font-size: 24px;
    font-weight: 700;
  }

  .primary-button,
  .secondary-button {
    min-height: 42px;
    border-radius: 10px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    white-space: nowrap;
    font-weight: 600;
  }

  .primary-button {
    border: 0;
    background: #ffc640;
    color: #1f1f1f;
    box-shadow: 0 4px 10px rgba(255, 190, 40, 0.18);
  }

  .primary-button:hover {
    background: #ffb820;
  }

  .primary-button:active {
    background: #eaa500;
  }

  .secondary-button {
    height: 42px;
    padding: 0 24px;
    border: 1px solid #e6ddcf;
    background: #fff;
    color: #333;
    box-shadow: none;
  }

  .secondary-button:hover {
    background: #fffcf5;
  }

  .secondary-button.large,
  .primary-button.large {
    min-width: 128px;
    height: 54px;
    padding: 0 30px;
    font-size: 17px;
  }

  h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
  }

  .archive-row:hover,
  .compress-tree-row:hover {
    background: #fffcf5;
  }

  .file-name {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 16px;
  }

  .file-name > span:last-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-badge {
    flex: 0 0 auto;
    width: 30px;
    height: 30px;
    border-radius: 6px;
    color: #fff;
    font-size: 14px;
    font-weight: 700;
    display: grid;
    place-items: center;
    position: relative;
  }

  .system-file-icon {
    flex: 0 0 auto;
    width: 30px;
    height: 30px;
    display: block;
    object-fit: contain;
  }

  .file-badge.zip,
  .file-badge.rar,
  .file-badge.archive {
    background: linear-gradient(180deg, #ffd978 0%, #ffc640 100%);
  }

  .file-badge.zip::after,
  .file-badge.rar::after,
  .file-badge.archive::after {
    content: "";
    position: absolute;
    left: 13px;
    top: 1px;
    width: 6px;
    height: 24px;
    background:
      linear-gradient(#384047 0 0) 0 0 / 3px 2px,
      linear-gradient(#384047 0 0) 3px 4px / 3px 2px,
      linear-gradient(#384047 0 0) 0 8px / 3px 2px,
      linear-gradient(#384047 0 0) 3px 12px / 3px 2px,
      linear-gradient(#384047 0 0) 0 16px / 3px 2px,
      linear-gradient(#384047 0 0) 3px 20px / 3px 2px;
    background-repeat: no-repeat;
  }

  .file-badge.word {
    background: linear-gradient(180deg, #5f9bff, #2e72dc);
  }

  .file-badge.pdf {
    background: linear-gradient(180deg, #ff6d67, #ee3535);
  }

  .file-badge.excel {
    background: linear-gradient(180deg, #33c778, #17a655);
  }

  .file-badge.image {
    border: 2px solid #4a97ff;
    background: #eef6ff;
    color: #4a97ff;
  }

  .file-badge.text {
    border: 1px solid #cfc8bd;
    background: #f3f1ed;
    color: #8f887d;
  }

  .file-badge.file {
    border: 1px solid #d7d0c6;
    background: #fffaf1;
    color: #7e766c;
  }

  .file-badge.mini {
    width: 22px;
    height: 22px;
    border-radius: 5px;
    font-size: 11px;
  }

  .file-badge.mini::after {
    left: 9px;
    top: 1px;
    transform: scale(0.72);
    transform-origin: top left;
  }

  .empty-row,
  .archive-empty {
    min-height: 86px;
    color: #999;
    display: grid;
    place-items: center;
    font-size: 14px;
  }

  .archive-empty {
    min-height: 240px;
    border-bottom: 1px solid #efeae2;
  }

  .extract-toolbar {
    height: 86px;
    flex: 0 0 auto;
    border-bottom: 1px solid #ede4d5;
    padding: 16px 24px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(300px, 420px);
    align-items: center;
    gap: 14px;
  }

  .archive-path {
    min-width: 0;
    display: grid;
    gap: 5px;
  }

  .archive-path strong {
    min-width: 0;
    font-size: 18px;
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-path span {
    color: #777;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-search-tools {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 86px 34px;
    align-items: center;
    gap: 8px;
  }

  .archive-search-field {
    min-width: 0;
    height: 34px;
    border: 1px solid #e7ded1;
    border-radius: 9px;
    background: #fff;
    color: #777;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
  }

  .archive-search-field:focus-within {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.14);
  }

  .archive-search-field input {
    width: 100%;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: #333;
    font-size: 13px;
  }

  .archive-search-field input::placeholder {
    color: #a59b90;
  }

  .archive-search-tools select {
    width: 100%;
    height: 34px;
    border: 1px solid #e7ded1;
    border-radius: 9px;
    background: #fff;
    color: #4f4942;
    outline: 0;
    padding: 0 26px 0 9px;
    font-size: 13px;
    appearance: none;
    background-image: linear-gradient(45deg, transparent 50%, #777 50%),
      linear-gradient(135deg, #777 50%, transparent 50%);
    background-position: calc(100% - 14px) 14px, calc(100% - 9px) 14px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  .archive-search-tools select:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.14);
  }

  .archive-info-button {
    width: 34px;
    height: 34px;
    border: 1px solid #e7ded1;
    border-radius: 9px;
    background: #fff;
  }

  .archive-info-button[aria-pressed="true"] {
    border-color: #f2c14e;
    background: #fff0c8;
    color: #1f1f1f;
  }

  .extract-grid {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(420px, 1fr);
  }

  .folder-icon {
    flex: 0 0 auto;
    display: inline-block;
    position: relative;
    background: linear-gradient(180deg, #ffd875, #ffc640);
  }

  .folder-icon::before {
    content: "";
    position: absolute;
    left: 2px;
    top: -5px;
    width: 12px;
    height: 7px;
    border-radius: 4px 4px 0 0;
    background: #ffd875;
  }

  .folder-icon {
    width: 24px;
    height: 18px;
    border-radius: 5px;
  }

  .folder-icon::before {
    width: 13px;
    height: 7px;
    top: -5px;
  }

  .archive-list-panel {
    min-width: 0;
    overflow: auto;
    background: #fffefb;
    outline: 0;
  }

  .archive-list-panel.archive-root-drop-target {
    box-shadow: inset 0 0 0 2px rgba(242, 177, 32, 0.34);
  }

  .archive-list-panel .file-name {
    gap: 8px;
  }

  .archive-file-cell {
    width: 100%;
    min-width: 0;
    min-height: 32px;
    border: 0;
    background: transparent;
    color: inherit;
    justify-content: flex-start;
    padding: 0 0 0 calc(var(--tree-depth, 0) * 14px);
    text-align: left;
    cursor: pointer;
  }

  .archive-row:not(.folder-row) .archive-file-cell {
    cursor: grab;
  }

  .archive-row:not(.folder-row) .archive-file-cell:active {
    cursor: grabbing;
  }

  .archive-name-copy {
    min-width: 0;
    display: block;
    line-height: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-name-copy > span,
  .archive-name-copy small {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-name-copy > span {
    display: block;
    color: #373737;
  }

  .archive-name-copy small {
    display: none;
    color: #9a938a;
    font-size: 10px;
    font-weight: 400;
  }

  .inline-badge {
    display: inline-flex;
    height: 16px;
    margin-left: 6px;
    padding: 0 5px;
    border-radius: 5px;
    background: #fff0c8;
    color: #8a5d00;
    vertical-align: 1px;
    font-size: 10px;
    font-weight: 700;
    line-height: 16px;
  }

  .inline-badge.muted {
    background: #f1ebe2;
    color: #8a8178;
  }

  .archive-list-panel .file-badge {
    width: 21px;
    height: 21px;
    border-radius: 5px;
    font-size: 10px;
  }

  .archive-list-panel .system-file-icon {
    width: 21px;
    height: 21px;
  }

  .archive-list-panel .file-badge.zip::after,
  .archive-list-panel .file-badge.rar::after,
  .archive-list-panel .file-badge.archive::after {
    left: 8px;
    top: 1px;
    transform: scale(0.68);
    transform-origin: top left;
  }

  .archive-list-panel .file-badge.image :global(svg) {
    width: 14px;
    height: 14px;
  }

  .archive-list-panel .folder-icon {
    width: 21px;
    height: 15px;
    border-radius: 4px;
  }

  .archive-list-panel .folder-icon::before {
    width: 11px;
    height: 6px;
    top: -4px;
  }

  .archive-table-row {
    margin-top: 2px;
    width: 100%;
    min-width: 0;
    min-height: 32px;
    border: 0;
    border-bottom: 0;
    background: #fffefb;
    color: #565656;
    display: grid;
    grid-template-columns: var(--archive-table-columns, 26px minmax(220px, 1fr) 82px 72px 146px);
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    text-align: left;
    font-size: 12px;
  }

  .archive-table-row > span,
  .archive-table-row > button {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-head {
    margin-top: 0;
    min-height: 34px;
    color: #777;
    font-weight: 500;
    background: #fffefb;
    position: sticky;
    top: 0;
    z-index: 2;
    box-shadow: 0 1px 0 #f2eadf;
  }

  .archive-head-cell {
    position: relative;
    height: 100%;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 5px;
    padding-right: 9px;
  }

  .archive-table-row > .archive-head-cell {
    overflow: visible;
  }

  .archive-sort-button {
    min-width: 0;
    max-width: 100%;
    height: 100%;
    border: 0;
    padding: 0;
    background: transparent;
    color: inherit;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font: inherit;
    cursor: pointer;
  }

  .archive-sort-button:hover {
    color: #36312a;
  }

  .archive-sort-button > span:first-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sort-chevron {
    flex: 0 0 auto;
    transition: transform 0.15s ease;
  }

  .sort-chevron.ascending {
    transform: rotate(180deg);
  }

  .column-resizer {
    position: absolute;
    top: 5px;
    right: -7px;
    z-index: 3;
    width: 14px;
    height: calc(100% - 10px);
    border: 0;
    border-radius: 5px;
    background: transparent;
    cursor: col-resize;
  }

  .column-resizer::after {
    content: "";
    position: absolute;
    top: 4px;
    bottom: 4px;
    left: 6px;
    width: 2px;
    border-radius: 999px;
    background: transparent;
  }

  .archive-head-cell:hover .column-resizer::after,
  .column-resizer:hover::after,
  .stage.resizing-columns .column-resizer::after {
    background: #e0d1bd;
  }

  .stage.resizing-columns,
  .stage.resizing-columns * {
    cursor: col-resize !important;
    user-select: none;
    -webkit-user-select: none;
  }

  .archive-row {
    user-select: none;
  }

  .archive-row.folder-row {
    background: #fffefb;
    color: #40382e;
    font-weight: 600;
  }

  .archive-row:hover {
    background: #fff7db;
  }

  .archive-row.selected {
    background: #ffedb3;
  }

  .archive-row.selected:hover {
    background: #ffedb3;
  }

  .archive-row.archive-drop-target {
    background: #ffe08a;
    box-shadow: inset 0 0 0 2px rgba(242, 177, 32, 0.42);
  }

  .archive-row:focus-visible {
    outline: 2px solid rgba(255, 198, 64, 0.58);
    outline-offset: -2px;
  }

  .check-cell {
    display: grid;
    place-items: center;
  }

  .check-cell input {
    width: 14px;
    height: 14px;
    accent-color: #ffc640;
  }

  .check-cell input[data-partial="true"] {
    outline: 1px solid rgba(122, 97, 48, 0.28);
    outline-offset: 1px;
  }

  .tree-toggle {
    width: 18px;
    height: 18px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #837463;
    display: grid;
    place-items: center;
  }

  .tree-toggle:hover {
    background: #fff0c8;
    color: #1f1f1f;
  }

  .archive-file-cell .tree-toggle {
    flex: 0 0 18px;
  }

  .archive-file-drag-handle {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: grab;
  }

  .archive-file-drag-handle:active {
    cursor: grabbing;
  }

  .icon-button {
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: #6f6a62;
    display: grid;
    place-items: center;
  }

  .icon-button:hover:not(:disabled) {
    background: #fff0c8;
    color: #1f1f1f;
  }

  .archive-info-backdrop {
    position: fixed;
    inset: 0;
    z-index: 38;
    background: rgba(45, 38, 28, 0.12);
    display: grid;
    justify-items: end;
  }

  .info-panel {
    padding: 18px 14px;
    background: #fffdf9;
    overflow: auto;
  }

  .archive-info-drawer {
    width: min(360px, calc(100vw - 28px));
    height: 100%;
    border-left: 1px solid #ede4d5;
    box-shadow: -18px 0 46px rgba(62, 48, 28, 0.16);
    animation: archive-info-slide-in 0.18s ease-out;
  }

  .archive-info-head {
    margin-bottom: 16px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
  }

  .info-panel h2 {
    font-size: 17px;
    margin: 0;
  }

  dl {
    margin: 0;
    display: grid;
    gap: 13px;
  }

  dl div {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 10px;
    color: #555;
    font-size: 12px;
  }

  .info-panel dd {
    min-width: 0;
    text-align: right;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .panel-action-grid {
    margin-top: 14px;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }

  .panel-action-grid .secondary-button {
    width: 100%;
    height: 32px;
    min-height: 0;
    border-radius: 8px;
    padding: 0 9px;
    font-size: 12px;
  }

  .compact-status {
    margin-top: 10px;
    padding: 8px 10px;
    border-radius: 8px;
    background: #f0fbf3;
    font-size: 12px;
    line-height: 1.35;
  }

  @keyframes archive-info-slide-in {
    from {
      transform: translateX(100%);
    }
    to {
      transform: translateX(0);
    }
  }

  dt,
  dd {
    margin: 0;
  }

  .compact-field {
    margin-top: 20px;
    display: grid;
    gap: 8px;
    color: #555;
    font-size: 13px;
  }

  .compact-field select {
    width: 100%;
    height: 34px;
    border: 1px solid #e7ded1;
    border-radius: 8px;
    background: #fff;
    color: #333;
    outline: 0;
    padding: 0 30px 0 10px;
    font-size: 13px;
    appearance: none;
    background-image: linear-gradient(45deg, transparent 50%, #777 50%),
      linear-gradient(135deg, #777 50%, transparent 50%);
    background-position: calc(100% - 16px) 14px, calc(100% - 11px) 14px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  .compact-field select:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.16);
  }

  .safety-note {
    margin-top: 16px;
    padding: 10px;
    border: 1px solid #eadfce;
    border-radius: 8px;
    background: #fff9ed;
    color: #665a48;
    display: grid;
    gap: 5px;
    font-size: 12px;
    line-height: 1.35;
  }

  .safety-note.danger {
    border-color: #f0b8a8;
    background: #fff3ef;
    color: #8a3d2b;
  }

  .safety-note strong {
    color: #36312a;
    font-size: 13px;
  }

  .safety-note small {
    color: inherit;
    opacity: 0.78;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-context-dismiss {
    position: fixed;
    inset: 0;
    z-index: 42;
    border: 0;
    background: transparent;
    padding: 0;
  }

  .archive-context-menu {
    position: fixed;
    z-index: 43;
    width: 220px;
    padding: 6px;
    border: 1px solid rgba(160, 135, 94, 0.24);
    border-radius: 10px;
    background: #fffdf9;
    box-shadow: 0 16px 42px rgba(62, 48, 28, 0.18);
    display: grid;
    gap: 2px;
  }

  .archive-context-menu button {
    height: 32px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: #39332d;
    text-align: left;
    padding: 0 10px;
    font-size: 13px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .archive-context-menu button:hover:not(:disabled) {
    background: #fff0c8;
  }

  .archive-context-menu button.danger {
    color: #b63b2b;
  }

  .archive-context-menu button:disabled {
    color: #b6aea5;
  }

  .context-separator {
    height: 1px;
    background: #efe6d8;
    margin: 4px 2px;
  }

  .context-open-with {
    display: grid;
    gap: 2px;
    padding: 2px 0 4px 10px;
  }

  .context-open-with button {
    height: 28px;
    color: #655b4f;
    font-size: 12px;
    font-weight: 600;
  }

  .context-status {
    min-height: 28px;
    display: flex;
    align-items: center;
    padding: 0 10px;
    color: #8f8478;
    font-size: 12px;
    font-weight: 600;
  }

  .app-panel-backdrop {
    position: fixed;
    inset: 0;
    z-index: 35;
    padding: 84px 24px 24px;
    background: rgba(45, 38, 28, 0.16);
    backdrop-filter: blur(6px);
    display: grid;
    place-items: start center;
  }

  .app-panel-window {
    width: min(660px, 100%);
    max-height: min(620px, calc(100vh - 108px));
    border: 1px solid rgba(160, 135, 94, 0.22);
    border-radius: 16px;
    background: #fffdf9;
    box-shadow: 0 24px 70px rgba(62, 48, 28, 0.2);
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto;
    overflow: hidden;
  }

  .app-panel-head {
    min-height: 72px;
    padding: 18px 20px;
    border-bottom: 1px solid #efe5d6;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 14px;
  }

  .app-panel-head h2 {
    font-size: 20px;
  }

  .app-panel-head p {
    margin: 5px 0 0;
    color: #8a8178;
    font-size: 13px;
  }

  .panel-close {
    background: #fff7e8;
  }

  .recent-window-content {
    min-height: 0;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    overflow: hidden;
  }

  .recent-search {
    padding: 14px 18px;
    border-bottom: 1px solid #efe5d6;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
  }

  .recent-search-field {
    min-width: 0;
    height: 38px;
    border: 1px solid #e7ded1;
    border-radius: 10px;
    background: #fff;
    color: #77808a;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 0 12px;
  }

  .recent-search-field:focus-within {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
  }

  .recent-search-field input {
    width: 100%;
    min-width: 0;
    height: 100%;
    border: 0;
    outline: 0;
    color: #333;
    background: transparent;
    font-size: 14px;
  }

  .recent-search-field input::placeholder {
    color: #aaa;
  }

  .recent-search-button {
    height: 38px;
    padding: 0 14px;
    border: 0;
    border-radius: 10px;
    background: #ffc640;
    color: #1f1f1f;
    font-size: 14px;
    font-weight: 700;
  }

  .recent-search-message {
    grid-column: 1 / -1;
    color: #8a8175;
    font-size: 12px;
    line-height: 1.3;
  }

  .recent-window-list {
    min-height: 0;
    overflow: auto;
  }

  .recent-window-row {
    min-width: 0;
    min-height: 48px;
    padding: 0 10px 0 14px;
    border-bottom: 1px solid #efeae2;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 56px 104px 30px;
    align-items: center;
    gap: 8px;
  }

  .recent-window-row:last-child {
    border-bottom: 0;
  }

  .recent-window-row:hover {
    background: #fffcf5;
  }

  .recent-open-button {
    min-width: 0;
    height: 100%;
    border: 0;
    background: transparent;
    color: inherit;
    display: contents;
    text-align: left;
  }

  .recent-window-row .file-name {
    min-width: 0;
    gap: 9px;
  }

  .recent-window-row .file-name > span:last-child {
    display: grid;
    gap: 2px;
  }

  .recent-window-row .file-badge {
    width: 24px;
    height: 24px;
    border-radius: 5px;
    font-size: 11px;
  }

  .recent-window-row .system-file-icon {
    width: 24px;
    height: 24px;
  }

  .recent-window-row strong,
  .recent-window-row small {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .recent-window-row strong {
    color: #333;
    font-size: 13px;
    font-weight: 650;
  }

  .recent-window-row small,
  .recent-open-button > span:nth-child(2),
  .recent-open-button > span:nth-child(3) {
    color: #8a8178;
    font-size: 11px;
  }

  .app-panel-actions {
    min-height: 72px;
    padding: 14px 20px 18px;
    border-top: 1px solid #efe5d6;
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 12px;
  }

  .settings-window-content {
    min-width: 0;
    overflow: auto;
    padding: 22px;
    display: grid;
    gap: 20px;
  }

  .settings-format-note {
    padding: 16px;
    border: 1px solid #efe5d6;
    border-radius: 10px;
    background: #fffcf7;
    display: grid;
    gap: 8px;
    color: #777;
    font-size: 13px;
  }

  .settings-format-note strong {
    color: #333;
    line-height: 1.45;
  }

  .permission-settings-note .secondary-button {
    justify-self: start;
    height: 36px;
    min-height: 36px;
    padding: 0 14px;
    font-size: 13px;
  }

  .permission-guide-backdrop {
    position: fixed;
    inset: 0;
    z-index: 58;
    padding: 14px;
    background: rgba(45, 38, 28, 0.24);
    backdrop-filter: blur(8px);
    display: grid;
    place-items: center;
  }

  .permission-guide-dialog {
    width: min(1040px, 100%);
    height: calc(100vh - 28px);
    max-height: calc(100vh - 28px);
    border: 1px solid rgba(160, 135, 94, 0.22);
    border-radius: 16px;
    background: #fffdf9;
    box-shadow: 0 24px 70px rgba(62, 48, 28, 0.22);
    overflow: hidden;
    display: grid;
    grid-template-rows: auto minmax(0, 1fr) auto auto;
  }

  .permission-guide-head {
    min-width: 0;
    padding: 20px 20px 18px;
    border-bottom: 1px solid #efe5d6;
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr) auto;
    align-items: center;
    gap: 14px;
  }

  .permission-guide-icon {
    width: 48px;
    height: 48px;
    border-radius: 13px;
    background: #fff0c8;
    color: #1f1f1f;
    display: grid;
    place-items: center;
  }

  .permission-guide-head h2 {
    margin: 0;
    color: #1f1f1f;
    font-size: 21px;
    line-height: 1.25;
  }

  .permission-guide-head p {
    margin: 5px 0 0;
    color: #777;
    font-size: 13px;
    line-height: 1.45;
  }

  .permission-steps {
    min-height: 0;
    padding: 12px 14px;
    overflow: hidden;
    display: grid;
    gap: 10px;
  }

  .onboarding-steps {
    grid-template-rows: minmax(0, 1fr);
    gap: 14px;
  }

  .onboarding-steps.last-guide-step {
    grid-template-rows: minmax(0, 1fr) auto;
  }

  .onboarding-image-frame {
    min-width: 0;
    min-height: 0;
    width: 100%;
    height: 100%;
    box-sizing: border-box;
    border: 1px solid #efe5d6;
    border-radius: 14px;
    background: #fff;
    background-image: var(--guide-image);
    background-position: center;
    background-repeat: no-repeat;
    background-size: contain;
    overflow: hidden;
    display: grid;
    place-items: center;
  }

  .onboarding-image-frame.missing {
    background-image: none;
  }

  .onboarding-image-probe {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }

  .onboarding-image-placeholder {
    min-width: 0;
    padding: 22px;
    color: #7b7065;
    text-align: center;
    display: grid;
    justify-items: center;
    gap: 8px;
  }

  .onboarding-image-placeholder strong {
    max-width: 100%;
    color: #2c2924;
    font-size: 15px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .onboarding-image-placeholder span {
    font-size: 12.5px;
    line-height: 1.4;
  }

  .permission-step {
    min-width: 0;
    padding: 13px;
    border: 1px solid #eee3d5;
    border-radius: 12px;
    background: #fff;
    display: grid;
    grid-template-columns: 36px minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
  }

  .permission-step-icon {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: #fff0c8;
    color: #1f1f1f;
    display: grid;
    place-items: center;
  }

  .permission-step strong {
    display: block;
    color: #24211d;
    font-size: 14px;
    line-height: 1.25;
  }

  .permission-step p {
    min-width: 0;
    margin: 5px 0 0;
    color: #7f756a;
    font-size: 12.5px;
    line-height: 1.45;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .permission-step .secondary-button {
    height: 34px;
    min-height: 34px;
    padding: 0 12px;
    border-radius: 9px;
    font-size: 12.5px;
  }

  .permission-guide-message {
    margin: 0 20px;
    padding: 10px 12px;
    border: 1px solid #efe5d6;
    border-radius: 10px;
    background: #fffcf7;
    color: #6f675d;
    font-size: 12.5px;
    line-height: 1.45;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .permission-guide-actions {
    min-height: 70px;
    padding: 14px 20px 18px;
    border-top: 1px solid #efe5d6;
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 10px;
  }

  .onboarding-dots {
    margin-right: auto;
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .onboarding-dots button {
    width: 8px;
    height: 8px;
    border: 0;
    border-radius: 999px;
    background: #d8ccbb;
    padding: 0;
  }

  .onboarding-dots button.active {
    width: 22px;
    background: #f2c14e;
  }

  .permission-guide-actions .primary-button,
  .permission-guide-actions .secondary-button {
    height: 38px;
    min-height: 38px;
    padding: 0 16px;
    border-radius: 10px;
    font-size: 13px;
  }

  .progress-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    padding: 24px;
    background: rgba(45, 38, 28, 0.24);
    backdrop-filter: blur(8px);
    display: grid;
    place-items: center;
  }

  .promise-drag-progress {
    position: fixed;
    right: 18px;
    bottom: 18px;
    z-index: 70;
    width: min(360px, calc(100vw - 36px));
    padding: 12px;
    border: 1px solid rgba(160, 135, 94, 0.2);
    border-radius: 14px;
    background: rgba(255, 253, 249, 0.96);
    box-shadow: 0 16px 42px rgba(62, 48, 28, 0.18);
    backdrop-filter: blur(12px);
    display: grid;
    grid-template-columns: 34px minmax(0, 1fr);
    align-items: center;
    gap: 10px;
    pointer-events: none;
  }

  .promise-drag-progress.done {
    border-color: rgba(31, 122, 66, 0.2);
  }

  .promise-drag-progress.failed {
    border-color: rgba(192, 57, 43, 0.22);
  }

  .promise-drag-icon {
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: #fff0c8;
    color: #1f1f1f;
    display: grid;
    place-items: center;
  }

  .promise-drag-icon :global(svg) {
    animation: progress-spin 1.1s linear infinite;
  }

  .promise-drag-progress.done .promise-drag-icon {
    background: #e7f6ed;
    color: #1f7a42;
  }

  .promise-drag-progress.failed .promise-drag-icon {
    background: #fff0ed;
    color: #c0392b;
  }

  .promise-drag-progress.done .promise-drag-icon :global(svg),
  .promise-drag-progress.failed .promise-drag-icon :global(svg) {
    animation: none;
  }

  .promise-drag-body {
    min-width: 0;
    display: grid;
    gap: 7px;
  }

  .promise-drag-head {
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .promise-drag-head strong {
    min-width: 0;
    color: #1f1f1f;
    font-size: 13px;
    line-height: 1.2;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .promise-drag-head span {
    flex: 0 0 auto;
    color: #8a7d70;
    font-size: 12px;
    font-weight: 700;
  }

  .promise-drag-body p {
    margin: 0;
    color: #736960;
    font-size: 12px;
    line-height: 1.35;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .promise-drag-meter {
    height: 7px;
    border-radius: 999px;
    background: #efe7dc;
    overflow: hidden;
  }

  .promise-drag-meter span {
    height: 100%;
    min-width: 4px;
    border-radius: inherit;
    background: linear-gradient(90deg, #ffc640, #f28d35);
    display: block;
    transition: width 0.24s ease;
  }

  .password-backdrop {
    z-index: 52;
  }

  .progress-dialog {
    width: min(520px, 100%);
    border: 1px solid rgba(160, 135, 94, 0.22);
    border-radius: 16px;
    background: #fffdf9;
    box-shadow: 0 24px 70px rgba(62, 48, 28, 0.22);
    padding: 24px;
    display: grid;
    gap: 18px;
  }

  .password-dialog {
    width: min(440px, 100%);
  }

  .destination-dialog {
    width: min(500px, 100%);
  }

  .password-form {
    display: grid;
    gap: 18px;
  }

  .destination-form {
    gap: 16px;
  }

  .destination-options {
    display: grid;
    gap: 10px;
  }

  .destination-option {
    min-width: 0;
    padding: 12px;
    border: 1px solid #eee3d5;
    border-radius: 12px;
    background: #fff;
    display: grid;
    gap: 9px;
  }

  .destination-option.active {
    border-color: #f2c14e;
    background: #fffaf0;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.12);
  }

  .destination-radio {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 9px;
    color: #333;
    font-size: 13px;
    font-weight: 700;
  }

  .destination-radio input {
    width: 15px;
    height: 15px;
    padding: 0;
    accent-color: #ffc640;
  }

  .destination-path-picker {
    min-width: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 82px;
    align-items: center;
    gap: 8px;
  }

  .destination-path-input {
    width: 100%;
    min-width: 0;
    height: 36px;
    border: 1px solid #e7ded1;
    border-radius: 9px;
    background: #fff;
    color: #333;
    outline: 0;
    padding: 0 11px;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .destination-path-input[readonly] {
    background: #fffcf7;
    color: #777;
  }

  .destination-path-input:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.16);
  }

  .destination-path-picker .browse-button {
    width: 82px;
    height: 36px;
    min-height: 36px;
    padding: 0 12px;
    border-radius: 9px;
    font-size: 12.5px;
  }

  .progress-dialog-head {
    display: grid;
    grid-template-columns: 48px minmax(0, 1fr);
    align-items: center;
    gap: 14px;
  }

  .progress-dialog-head h2 {
    margin: 0;
    color: #1f1f1f;
    font-size: 21px;
    line-height: 1.25;
  }

  .progress-dialog-head p {
    margin: 5px 0 0;
    color: #777;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-icon {
    width: 48px;
    height: 48px;
    border-radius: 13px;
    background: #fff0c8;
    color: #1f1f1f;
    display: grid;
    place-items: center;
  }

  .progress-icon :global(svg) {
    animation: progress-spin 1.1s linear infinite;
  }

  .password-icon {
    background: #fff0c8;
    color: #1f1f1f;
  }

  .password-icon :global(svg) {
    animation: none;
  }

  .progress-icon.done {
    background: #e7f6ed;
    color: #1f7a42;
  }

  .progress-icon.failed {
    background: #fff0ed;
    color: #c0392b;
  }

  .progress-icon.done :global(svg),
  .progress-icon.failed :global(svg) {
    animation: none;
  }

  .progress-group {
    display: grid;
    gap: 16px;
  }

  .progress-line {
    display: grid;
    gap: 8px;
  }

  .progress-line-head,
  .progress-line-foot {
    min-width: 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 14px;
  }

  .progress-line-head {
    color: #333;
    font-size: 14px;
    font-weight: 600;
  }

  .progress-line-head strong {
    color: #1f1f1f;
    font-size: 16px;
  }

  .progress-line-foot {
    color: #857b72;
    font-size: 12px;
  }

  .progress-line-foot span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-line-foot span:last-child {
    flex: 0 0 auto;
  }

  .progress-meter {
    height: 12px;
    border-radius: 999px;
    background: #efe7dc;
    overflow: hidden;
  }

  .progress-fill {
    height: 100%;
    min-width: 4px;
    border-radius: inherit;
    background: linear-gradient(90deg, #ffc640, #f28d35);
    display: block;
    transition: width 0.24s ease;
  }

  .progress-fill.current {
    background: linear-gradient(90deg, #7aa7ff, #326fd8);
  }

  .progress-fill.total {
    background: linear-gradient(90deg, #ffc640, #f28d35);
  }

  .password-field {
    display: grid;
    gap: 8px;
    color: #6f675f;
    font-size: 13px;
    font-weight: 650;
  }

  .password-field input {
    width: 100%;
    height: 46px;
    border: 1px solid #e7ded1;
    border-radius: 10px;
    background: #fff;
    color: #333;
    outline: 0;
    padding: 0 13px;
    font-size: 15px;
  }

  .password-field input:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
  }

  .password-message {
    margin: 0;
    color: #9a6a00;
    font-size: 13px;
    line-height: 1.45;
  }

  .progress-details {
    margin: 0;
    display: grid;
    gap: 10px;
  }

  .progress-details div {
    min-width: 0;
    display: grid;
    grid-template-columns: 76px minmax(0, 1fr);
    gap: 12px;
    color: #555;
    font-size: 14px;
  }

  .progress-details dt {
    color: #8a8178;
  }

  .progress-details dd {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .progress-message {
    min-height: 22px;
    margin: 0;
    color: #777;
    font-size: 14px;
    line-height: 1.55;
  }

  .progress-message.progress-success {
    color: #1f7a42;
  }

  .progress-message.progress-error {
    color: #c0392b;
  }

  .progress-actions {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 12px;
  }

  .secondary-button.danger {
    color: #b72f25;
  }

  .secondary-button.danger:hover {
    border-color: #efc5bd;
    background: #fff4f1;
  }

  .spinning-icon {
    animation: progress-spin 1.1s linear infinite;
  }

  @keyframes progress-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .compress-body {
    flex: 1;
    min-height: 0;
    padding: 26px 30px 18px;
    display: grid;
    grid-template-columns: minmax(300px, 1fr) minmax(260px, 300px);
    gap: 10px;
    overflow-x: auto;
    overflow-y: hidden;
  }

  .compress-left {
    min-width: 300px;
    min-height: 0;
    padding-right: 4px;
    display: grid;
    grid-template-rows: auto minmax(280px, 1fr) auto;
    align-content: stretch;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-gutter: auto;
    overscroll-behavior: contain;
  }

  .drop-zone {
    width: 100%;
    min-height: 54px;
    padding: 8px 14px;
    border: 1.5px dashed #e4d7c5;
    border-radius: 12px;
    background: #fffdf9;
    color: #555;
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    grid-template-rows: auto auto;
    align-items: center;
    column-gap: 12px;
    row-gap: 2px;
    text-align: left;
    cursor: pointer;
  }

  .drop-zone:hover {
    border-color: #f2c14e;
    background: #fffaf1;
  }

  .drop-zone:focus-visible {
    outline: 2px solid rgba(255, 198, 64, 0.58);
    outline-offset: 2px;
  }

  .drop-zone strong {
    align-self: end;
    font-size: 14px;
    font-weight: 700;
  }

  .drop-hint {
    align-self: start;
    color: #777;
    font-size: 12px;
  }

  .folder-add {
    grid-row: 1 / 3;
    width: 36px;
    height: 36px;
    color: #ffc640;
    position: relative;
    display: grid;
    place-items: center;
  }

  .folder-add :global(svg:last-child) {
    position: absolute;
    color: #1f1f1f;
    width: 16px;
    height: 16px;
  }

  .folder-add :global(svg:first-child) {
    width: 32px;
    height: 32px;
  }

  .settings-panel .drop-zone {
    min-height: 56px;
    padding: 8px 12px 9px;
    border-radius: 10px;
    column-gap: 10px;
    row-gap: 1px;
    align-content: center;
  }

  .settings-panel .drop-zone strong {
    font-size: 13px;
    line-height: 1.2;
  }

  .settings-panel .drop-hint {
    font-size: 11.5px;
    line-height: 1.25;
  }

  .settings-panel .folder-add {
    width: 32px;
    height: 32px;
  }

  .settings-panel .folder-add :global(svg:first-child) {
    width: 29px;
    height: 29px;
  }

  .settings-panel .folder-add :global(svg:last-child) {
    width: 14px;
    height: 14px;
  }

  .selected-summary {
    margin-top: 0;
    color: #555;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 13px;
  }

  .selected-list {
    margin-top: 8px;
    min-width: 0;
    border: 1px solid #efe5d6;
    border-radius: 10px;
    background: #fff;
    overflow: hidden;
  }

  .compress-tree {
    width: 100%;
    min-width: 0;
    min-height: 280px;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-gutter: auto;
    overscroll-behavior: contain;
  }

  .selected-list .file-badge.archive::after {
    left: 8px;
    top: 1px;
    transform: scale(0.68);
    transform-origin: top left;
  }

  .compress-tree .file-badge {
    width: 21px;
    height: 21px;
    border-radius: 5px;
    font-size: 10px;
  }

  .compress-tree .system-file-icon {
    width: 21px;
    height: 21px;
  }

  .compress-tree .file-badge.zip::after,
  .compress-tree .file-badge.rar::after,
  .compress-tree .file-badge.archive::after {
    left: 8px;
    top: 1px;
    transform: scale(0.68);
    transform-origin: top left;
  }

  .compress-tree .file-badge.image :global(svg) {
    width: 14px;
    height: 14px;
  }

  .compress-tree .folder-icon {
    width: 21px;
    height: 15px;
    border-radius: 4px;
  }

  .compress-tree .folder-icon::before {
    width: 11px;
    height: 6px;
    top: -4px;
  }

  .compress-tree-head,
  .compress-tree-row {
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
    min-height: 32px;
    padding: 0 10px;
    border-bottom: 1px solid #efeae2;
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr) 72px;
    align-items: center;
    gap: 6px;
    color: #555;
    font-size: 12.5px;
  }

  .compress-tree-head {
    min-height: 34px;
    background: #fffaf1;
    color: #8a8178;
    font-size: 12px;
    font-weight: 700;
    position: sticky;
    top: 0;
    z-index: 2;
  }

  .compress-tree-row:last-child {
    border-bottom: 0;
  }

  .compress-tree-row {
    user-select: none;
  }

  .compress-tree-row.selected {
    background: #fff7df;
  }

  .compress-file-cell {
    width: 100%;
    min-width: 0;
    min-height: 32px;
    gap: 7px;
    padding: 0 0 0 calc(var(--tree-depth) * 10px);
    border: 0;
    background: transparent;
    color: #333;
    text-align: left;
  }

  .compress-file-cell .tree-toggle {
    flex: 0 0 16px;
    width: 16px;
    height: 16px;
  }

  .visual-toggle {
    color: #8a8178;
    display: grid;
    place-items: center;
  }

  .spacer-toggle {
    width: 16px;
    height: 16px;
  }

  .compress-name-copy {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .compress-size-cell {
    color: #8a8178;
    font-size: 12px;
    text-align: right;
    white-space: nowrap;
  }

  .total-size {
    margin-top: 10px;
    color: #777;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
  }

  .total-size :global(svg) {
    color: #ffc640;
  }

  .clear-button {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    margin-left: auto;
  }

  .settings-panel {
    min-width: 260px;
    min-height: 0;
    padding-right: 4px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
    overflow-x: hidden;
    scrollbar-gutter: stable;
    overscroll-behavior: contain;
  }

  .settings-panel .field-block {
    gap: 7px;
    font-size: 13px;
  }

  .settings-panel .field-block input {
    height: 40px;
    padding: 0 12px;
    font-size: 13px;
  }

  .settings-panel .input-group,
  .settings-panel .path-picker {
    height: 40px;
    border-radius: 8px;
  }

  .settings-panel .input-group {
    grid-template-columns: minmax(0, 1fr) 98px;
    gap: 0;
    border: 0;
    background: transparent;
    overflow: visible;
  }

  .settings-panel .input-group:focus-within {
    border-color: transparent;
    box-shadow: none;
  }

  .settings-panel .input-group input {
    min-width: 0;
    border: 1px solid #e7ded1;
    border-right: 0;
    border-radius: 8px 0 0 8px;
    background: #fff;
  }

  .settings-panel .input-group input:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
    position: relative;
    z-index: 1;
  }

  .settings-panel .extension-select {
    width: 98px;
    height: 40px;
    padding: 0 24px 0 10px;
    border: 1px solid #e7ded1;
    border-radius: 0 8px 8px 0;
    background-position: calc(100% - 14px) 16px, calc(100% - 9px) 16px;
    box-shadow: none;
    font-size: 13px;
  }

  .settings-panel .extension-select:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
    position: relative;
    z-index: 2;
  }

  .settings-panel .path-picker {
    grid-template-columns: minmax(0, 1fr) 82px;
    gap: 8px;
  }

  .settings-panel .path-picker input,
  .settings-panel .field-block .plain-input,
  .settings-panel .split-size input {
    height: 40px;
    border-radius: 8px;
    font-size: 13px;
  }

  .settings-panel .browse-button {
    width: 82px;
    height: 40px;
    min-height: 40px;
    border-radius: 8px;
    font-size: 13px;
  }

  .field-block {
    display: grid;
    gap: 11px;
    color: #333;
    font-size: 16px;
    font-weight: 500;
  }

  .field-block input {
    width: 100%;
    height: 48px;
    border: 0;
    outline: 0;
    background: transparent;
    color: #333;
    padding: 0 16px;
    font-size: 16px;
  }

  .field-block input::placeholder {
    color: #b0b0b0;
  }

  .input-group,
  .path-picker,
  .select-field {
    height: 48px;
    border: 1px solid #e7ded1;
    border-radius: 10px;
    background: #fff;
    display: grid;
    align-items: center;
  }

  .input-group:focus-within,
  .select-field:focus-within {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
  }

  .input-group {
    grid-template-columns: minmax(0, 1fr) 122px;
  }

  .extension,
  .extension-select {
    height: 48px;
    border-left: 1px solid #e7ded1;
    border-top: 0;
    border-right: 0;
    border-bottom: 0;
    background: #fffcf7;
    border-radius: 0 10px 10px 0;
    color: #555;
    font-size: 15px;
  }

  .extension {
    display: grid;
    place-items: center;
  }

  .extension-select {
    width: 122px;
    padding: 0 28px 0 12px;
    outline: 0;
    appearance: none;
    background-image: linear-gradient(45deg, transparent 50%, #777 50%),
      linear-gradient(135deg, #777 50%, transparent 50%);
    background-position: calc(100% - 16px) 20px, calc(100% - 11px) 20px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  .extension-select:focus {
    background-color: #fff8e8;
  }

  .path-picker {
    grid-template-columns: minmax(0, 1fr) 106px;
    gap: 14px;
    border: 0;
    background: transparent;
  }

  .path-picker input {
    border: 1px solid #e7ded1;
    border-radius: 10px;
    background: #fff;
  }

  .path-picker input:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
  }

  .browse-button {
    width: 106px;
    height: 48px;
    padding: 0;
  }

  .field-block .plain-input,
  .format-select,
  .split-size input {
    height: 48px;
    border: 1px solid #e7ded1;
    border-radius: 10px;
    background: #fff;
    outline: 0;
    color: #333;
    font-size: 15px;
  }

  .field-block .plain-input:focus,
  .format-select:focus,
  .split-size input:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
  }

  .format-select {
    width: 100%;
    padding: 0 36px 0 14px;
    appearance: none;
    background-image: linear-gradient(45deg, transparent 50%, #777 50%),
      linear-gradient(135deg, #777 50%, transparent 50%);
    background-position: calc(100% - 18px) 20px, calc(100% - 13px) 20px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  .range-control {
    display: grid;
    gap: 10px;
  }

  .settings-panel .range-control {
    gap: 6px;
  }

  .range-head {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 10px;
    color: #8a8178;
    font-size: 13px;
    font-weight: 500;
  }

  .settings-panel .range-head {
    gap: 8px;
    font-size: 12px;
  }

  .range-head strong {
    min-width: 34px;
    height: 28px;
    border: 1px solid #e6ddcf;
    border-radius: 8px;
    background: #fff;
    color: #333;
    display: grid;
    place-items: center;
    font-size: 14px;
  }

  .settings-panel .range-head strong {
    min-width: 30px;
    height: 24px;
    border-radius: 7px;
    font-size: 12px;
  }

  .range-head span:last-child {
    text-align: right;
  }

  .range-control input[type="range"] {
    height: 28px;
    padding: 0;
    accent-color: #f2c14e;
  }

  .settings-panel .range-control input[type="range"] {
    height: 22px;
  }

  .switch-row {
    height: 40px;
    display: inline-flex;
    align-items: center;
    gap: 10px;
    color: #555;
    font-size: 14px;
    font-weight: 500;
  }

  .settings-panel .switch-row {
    height: 30px;
    gap: 8px;
    font-size: 13px;
  }

  .switch-row input {
    width: 18px;
    height: 18px;
    padding: 0;
    accent-color: #f2c14e;
  }

  .settings-panel .switch-row input {
    width: 16px;
    height: 16px;
  }

  .split-size {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    color: #777;
    font-size: 14px;
    font-weight: 500;
  }

  .settings-panel .split-size {
    gap: 8px;
    font-size: 13px;
  }

  .split-size input {
    padding: 0 12px;
  }

  .compact-field-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 8px;
  }

  .mini-field {
    min-width: 0;
    display: grid;
    gap: 5px;
    color: #6f665e;
    font-size: 12px;
    font-weight: 600;
  }

  .mini-field input,
  .mini-field select {
    width: 100%;
    height: 38px;
    border: 1px solid #e7ded1;
    border-radius: 8px;
    background: #fff;
    color: #333;
    outline: 0;
    padding: 0 10px;
    font-size: 13px;
  }

  .mini-field select {
    appearance: none;
    padding-right: 26px;
    background-image: linear-gradient(45deg, transparent 50%, #777 50%),
      linear-gradient(135deg, #777 50%, transparent 50%);
    background-position: calc(100% - 15px) 16px, calc(100% - 10px) 16px;
    background-size: 5px 5px, 5px 5px;
    background-repeat: no-repeat;
  }

  .mini-field input:focus,
  .mini-field select:focus {
    border-color: #f2c14e;
    box-shadow: 0 0 0 3px rgba(255, 198, 64, 0.18);
  }

  .label-with-help {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }

  .status-area {
    min-height: 40px;
    color: #777;
    font-size: 13px;
    line-height: 1.5;
  }

  .settings-panel .status-area {
    min-height: 30px;
    font-size: 12px;
    line-height: 1.35;
  }

  .error-text {
    color: #c0392b;
  }

  .success-text {
    color: #1f7a42;
  }

  @media (max-width: 1080px) {
    .main-titlebar {
      grid-template-columns: minmax(0, 1fr) auto;
      padding-left: 92px;
    }

    .home-layout {
      grid-template-columns: 1fr;
    }

    .extract-grid {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 900px) {
    .stage,
    .stage.compact {
      padding: 0;
    }

    .app-shell,
    .task-shell {
      width: 100%;
      height: 100vh;
      border-radius: 0;
    }

    .main-titlebar {
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 12px;
    }

    .home-layout {
      grid-template-columns: 1fr;
    }

    .home-content {
      padding: 24px;
    }

    .extract-toolbar {
      grid-template-columns: minmax(0, 1fr) minmax(270px, 360px);
      height: 76px;
      gap: 14px;
      padding: 14px 20px;
    }

    .extract-grid {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 520px) {
    .main-titlebar {
      padding-left: 74px;
      padding-right: 18px;
    }

    .task-titlebar {
      padding-left: 92px;
      padding-right: 18px;
    }

    .task-title {
      font-size: 16px;
    }

    .app-tabs button {
      min-width: 40px;
      padding: 0 9px;
    }

    .app-tabs button > span {
      display: none;
    }

    .app-panel-backdrop {
      padding: 74px 12px 12px;
    }

    .app-panel-window {
      max-height: calc(100vh - 86px);
    }

    .permission-guide-backdrop {
      padding: 8px;
    }

    .permission-guide-dialog {
      height: calc(100vh - 16px);
      max-height: calc(100vh - 16px);
    }

    .permission-guide-actions {
      flex-wrap: wrap;
    }

    .onboarding-dots {
      width: 100%;
      justify-content: center;
      margin-right: 0;
      order: -1;
    }

    .permission-guide-head {
      grid-template-columns: 42px minmax(0, 1fr) auto;
      padding: 16px;
      gap: 11px;
    }

    .permission-guide-icon {
      width: 42px;
      height: 42px;
      border-radius: 12px;
    }

    .permission-guide-head h2 {
      font-size: 18px;
    }

    .permission-guide-head p {
      font-size: 12px;
    }

    .permission-steps {
      padding: 14px;
    }

    .permission-step {
      grid-template-columns: 34px minmax(0, 1fr);
    }

    .permission-step .secondary-button {
      grid-column: 2;
      justify-self: start;
    }

    .permission-guide-message {
      margin: 0 14px;
      white-space: normal;
    }

    .permission-guide-actions {
      padding: 12px 14px 14px;
    }

    .recent-window-row {
      grid-template-columns: minmax(0, 1fr) 50px 36px;
    }

    .recent-window-row .recent-open-button > span:nth-child(3) {
      display: none;
    }

    .home-content {
      padding: 20px 18px;
    }

    .hero-actions {
      width: min(100%, 380px);
      grid-template-columns: repeat(2, minmax(0, 180px));
      gap: 16px;
    }

    .action-card {
      padding: 20px;
    }

    .archive-table-row {
      grid-template-columns: 26px minmax(0, 1fr);
    }

    .archive-table-row > span:nth-child(3),
    .archive-table-row > span:nth-child(4),
    .archive-table-row > span:nth-child(5) {
      display: none;
    }

  }
</style>
