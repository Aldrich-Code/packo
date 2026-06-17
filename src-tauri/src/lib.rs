use chrono::{DateTime, Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    ffi::{OsStr, OsString},
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager};

static EXTRACT_TASK_COUNTER: AtomicU64 = AtomicU64::new(1);
static WORK_WINDOW_COUNTER: AtomicU64 = AtomicU64::new(1);
const SUPPORTED_EXTRACT_FORMATS: &[&str] = &[
    "ZIP", "RAR", "7Z", "GZ", "GZIP", "BZ2", "BZIP2", "XZ", "TAR", "TGZ", "TBZ", "TXZ", "LZH",
    "LHA", "Z", "ZSTD", "LZMA", "LZMA2", "LZ4", "ISO",
];
const SUPPORTED_EXTRACT_FORMATS_LABEL: &str =
    "ZIP、RAR、7Z、GZ/GZIP、BZ2/BZIP2、XZ、TAR、TGZ、TBZ、TXZ、LZH/LHA、Z、ZSTD、LZMA/LZMA2、LZ4、ISO、DOCX/PPTX/XLSX 等 ZIP 容器、分卷压缩包";
const SUPPORTED_COMPRESS_FORMATS_LABEL: &str =
    "ZIP、7Z、TAR、TGZ、TBZ、TXZ、GZ/GZIP、BZ2/BZIP2、XZ、Z、ZSTD、LZMA/LZMA2、LZ4";

#[cfg(target_os = "macos")]
mod macos_file_promise_drag {
    use super::{
        extract_archive_entry_to_promised_item, ArchivePromiseDragItem, ArchivePromiseDragProgress,
    };
    use block2::DynBlock;
    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
        AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSApp, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
        NSDraggingSource, NSEvent, NSEventModifierFlags, NSEventType, NSFilePromiseProvider,
        NSFilePromiseProviderDelegate, NSImage, NSPasteboardWriting, NSView, NSWorkspace,
    };
    use objc2_foundation::{NSError, NSMutableArray, NSPoint, NSRect, NSSize, NSString, NSURL};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{path::PathBuf, ptr, sync::Arc};
    use tauri::Emitter;

    type PromiseDragProgressEmitter = Arc<dyn Fn(ArchivePromiseDragProgress) + Send + Sync>;

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = AnyThread]
        #[name = "CardiganZipArchiveFilePromiseDelegate"]
        #[ivars = ArchiveFilePromiseDelegateIvars]
        struct ArchiveFilePromiseDelegate;

        unsafe impl NSObjectProtocol for ArchiveFilePromiseDelegate {}

        unsafe impl NSFilePromiseProviderDelegate for ArchiveFilePromiseDelegate {
            #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
            unsafe fn file_name_for_type(
                &self,
                _file_promise_provider: &NSFilePromiseProvider,
                _file_type: &NSString,
            ) -> Retained<NSString> {
                NSString::from_str(&self.ivars().promised_name)
            }

            #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
            unsafe fn write_promise(
                &self,
                _file_promise_provider: &NSFilePromiseProvider,
                url: &NSURL,
                completion_handler: &DynBlock<dyn Fn(*mut NSError)>,
            ) {
                let ivars = self.ivars();
                (ivars.emit_progress)(ArchivePromiseDragProgress {
                    drag_id: ivars.drag_id.clone(),
                    status: "running".to_string(),
                    total: ivars.total_items,
                    completed: ivars.item_index,
                    current_item: ivars.promised_name.clone(),
                    message: format!("正在写入 {}", ivars.promised_name),
                    error: None,
                });
                let result = promised_output_path(url, &ivars.promised_name, ivars.is_dir)
                    .and_then(|output_path| {
                        extract_archive_entry_to_promised_item(
                            &ivars.archive_path,
                            &ivars.entry_path,
                            &output_path,
                            ivars.is_dir,
                            ivars.password.as_deref(),
                        )
                        .map(|_| ())
                    });

                match result {
                    Ok(()) => {
                        (ivars.emit_progress)(ArchivePromiseDragProgress {
                            drag_id: ivars.drag_id.clone(),
                            status: "completed".to_string(),
                            total: ivars.total_items,
                            completed: ivars.item_index + 1,
                            current_item: ivars.promised_name.clone(),
                            message: format!("已写入 {}", ivars.promised_name),
                            error: None,
                        });
                        completion_handler.call((ptr::null_mut(),));
                    }
                    Err(message) => {
                        eprintln!("file promise extraction failed: {message}");
                        (ivars.emit_progress)(ArchivePromiseDragProgress {
                            drag_id: ivars.drag_id.clone(),
                            status: "failed".to_string(),
                            total: ivars.total_items,
                            completed: ivars.item_index,
                            current_item: ivars.promised_name.clone(),
                            message: message.clone(),
                            error: Some(message.clone()),
                        });
                        let error = promise_error();
                        let error_ptr = (&*error) as *const NSError as *mut NSError;
                        completion_handler.call((error_ptr,));
                    }
                }
            }
        }
    );

    struct ArchiveFilePromiseDelegateIvars {
        archive_path: PathBuf,
        entry_path: String,
        promised_name: String,
        is_dir: bool,
        password: Option<String>,
        drag_id: String,
        item_index: usize,
        total_items: usize,
        emit_progress: PromiseDragProgressEmitter,
    }

    impl ArchiveFilePromiseDelegate {
        fn new(
            archive_path: PathBuf,
            entry_path: String,
            promised_name: String,
            is_dir: bool,
            password: Option<String>,
            drag_id: String,
            item_index: usize,
            total_items: usize,
            emit_progress: PromiseDragProgressEmitter,
        ) -> Retained<Self> {
            let this = Self::alloc().set_ivars(ArchiveFilePromiseDelegateIvars {
                archive_path,
                entry_path,
                promised_name,
                is_dir,
                password,
                drag_id,
                item_index,
                total_items,
                emit_progress,
            });
            unsafe { msg_send![super(this), init] }
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "CardiganZipPromiseDragSource"]
        #[ivars = PromiseDragSourceIvars]
        struct PromiseDragSource;

        unsafe impl NSObjectProtocol for PromiseDragSource {}

        unsafe impl NSDraggingSource for PromiseDragSource {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            unsafe fn dragging_session_operation_mask(
                &self,
                session: &NSDraggingSession,
                _context: NSDraggingContext,
            ) -> NSDragOperation {
                session.setAnimatesToStartingPositionsOnCancelOrFail(true);
                NSDragOperation::Copy
            }
        }
    );

    struct PromiseDragSourceIvars;

    impl PromiseDragSource {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(PromiseDragSourceIvars);
            unsafe { msg_send![super(this), init] }
        }
    }

    pub fn start_archive_entry_promise_drag<R: tauri::Runtime>(
        window: &tauri::Window<R>,
        archive_path: PathBuf,
        entry_path: String,
        promised_name: String,
        is_dir: bool,
        password: Option<String>,
    ) -> Result<(), String> {
        start_archive_entries_promise_drag(
            window,
            archive_path,
            vec![ArchivePromiseDragItem {
                entry_path,
                promised_name,
                is_dir,
            }],
            password,
        )
    }

    pub fn start_archive_entries_promise_drag<R: tauri::Runtime>(
        window: &tauri::Window<R>,
        archive_path: PathBuf,
        items: Vec<ArchivePromiseDragItem>,
        password: Option<String>,
    ) -> Result<(), String> {
        if items.is_empty() {
            return Err("请选择要拖出的项目。".to_string());
        }

        let handle = window
            .window_handle()
            .map_err(|_| "无法获取当前窗口句柄。".to_string())?;
        let RawWindowHandle::AppKit(appkit_window) = handle.as_raw() else {
            return Err("当前窗口不支持 macOS 文件拖拽。".to_string());
        };

        unsafe {
            let mtm = MainThreadMarker::new_unchecked();
            let ns_view = &*(appkit_window.ns_view.as_ptr() as *const NSView);
            let ns_window = ns_view
                .window()
                .ok_or_else(|| "无法获取当前窗口。".to_string())?;
            let content_view = ns_window
                .contentView()
                .ok_or_else(|| "无法获取当前窗口内容视图。".to_string())?;
            let current_position = ns_window.mouseLocationOutsideOfEventStream();
            let drag_id = format!(
                "promise-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis())
                    .unwrap_or(0)
            );
            let total_items = items.len();
            let emit_window = window.clone();
            let emit_progress: PromiseDragProgressEmitter = Arc::new(move |payload| {
                let _ = emit_window.emit("archive-promise-drag-progress", payload);
            });

            let dragging_items = NSMutableArray::new();
            for (index, item) in items.into_iter().enumerate() {
                let drag_image = drag_image_for_name(&item.promised_name, item.is_dir);
                let image_size = drag_image.size();
                let offset = (index.min(5) as f64) * 3.0;
                let image_rect = NSRect::new(
                    NSPoint::new(
                        current_position.x - image_size.width / 2.0 + offset,
                        current_position.y - image_size.height / 2.0 - offset,
                    ),
                    image_size,
                );

                let delegate = ArchiveFilePromiseDelegate::new(
                    archive_path.clone(),
                    item.entry_path,
                    item.promised_name.clone(),
                    item.is_dir,
                    password.clone(),
                    drag_id.clone(),
                    index,
                    total_items,
                    Arc::clone(&emit_progress),
                );
                let file_type =
                    NSString::from_str(file_promise_type(&item.promised_name, item.is_dir));
                let provider = NSFilePromiseProvider::initWithFileType_delegate(
                    NSFilePromiseProvider::alloc(),
                    &file_type,
                    ProtocolObject::from_ref(&*delegate),
                );

                // NSFilePromiseProvider keeps its delegate weakly. Store the delegate
                // in userInfo so it stays alive for Finder's writePromise callback.
                let retained_delegate: Retained<AnyObject> = delegate.into_super().into_super();
                provider.setUserInfo(Some(&retained_delegate));

                let drag_item = NSDraggingItem::initWithPasteboardWriter(
                    NSDraggingItem::alloc(),
                    &ProtocolObject::<dyn NSPasteboardWriting>::from_retained(provider),
                );
                drag_item.setDraggingFrame_contents(image_rect, Some(&*drag_image));
                dragging_items.addObject(&*drag_item);
            }

            let current_event = NSApp(mtm).currentEvent();
            let timestamp = current_event.map(|event| event.timestamp()).unwrap_or(0.0);
            let window_number = ns_window.windowNumber();
            let drag_event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDragged,
                current_position,
                NSEventModifierFlags::empty(),
                timestamp,
                window_number,
                None,
                0,
                1,
                1.0,
            )
            .ok_or_else(|| "无法创建原生拖拽事件。".to_string())?;

            let source = PromiseDragSource::new(mtm);
            let source = ProtocolObject::<dyn NSDraggingSource>::from_retained(source);
            content_view.beginDraggingSessionWithItems_event_source(
                &dragging_items,
                &drag_event,
                &source,
            );
        }

        Ok(())
    }

    fn promised_output_path(
        url: &NSURL,
        promised_name: &str,
        is_dir: bool,
    ) -> Result<PathBuf, String> {
        let path = url
            .path()
            .ok_or_else(|| "Finder 未提供有效目标路径。".to_string())?;
        let path = PathBuf::from(path.to_string());

        if path.is_dir() {
            return Ok(if is_dir {
                unique_folder_path(&path, promised_name)
            } else {
                super::unique_file_path(&path, promised_name)
            });
        }

        if path.file_name().is_some() {
            return Ok(path);
        }

        Err("Finder 未提供有效目标路径。".to_string())
    }

    #[allow(deprecated)]
    fn drag_image_for_name(name: &str, is_dir: bool) -> Retained<NSImage> {
        let file_type = if is_dir {
            "public.folder"
        } else {
            std::path::Path::new(name)
                .extension()
                .and_then(|extension| extension.to_str())
                .filter(|extension| !extension.is_empty())
                .unwrap_or("public.data")
        };
        let image = NSWorkspace::sharedWorkspace().iconForFileType(&NSString::from_str(file_type));
        image.setSize(NSSize::new(32.0, 32.0));
        image
    }

    fn file_promise_type(name: &str, is_dir: bool) -> &'static str {
        if is_dir {
            return "public.folder";
        }
        match std::path::Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "txt" | "md" | "csv" | "log" => "public.plain-text",
            "json" => "public.json",
            "pdf" => "com.adobe.pdf",
            "png" => "public.png",
            "jpg" | "jpeg" => "public.jpeg",
            "gif" => "com.compuserve.gif",
            "webp" => "org.webmproject.webp",
            "zip" => "public.zip-archive",
            "tar" => "public.tar-archive",
            "gz" => "org.gnu.gnu-zip-archive",
            "doc" | "docx" => "com.microsoft.word.doc",
            "xls" | "xlsx" => "com.microsoft.excel.xls",
            _ => "public.data",
        }
    }

    fn unique_folder_path(output_dir: &std::path::Path, folder_name: &str) -> PathBuf {
        let safe_name = super::sanitize_path_segment(folder_name);
        let mut candidate = output_dir.join(&safe_name);
        let mut index = 2;

        while candidate.exists() {
            candidate = output_dir.join(format!("{safe_name} {index}"));
            index += 1;
        }

        candidate
    }

    fn promise_error() -> Retained<NSError> {
        let domain = NSString::from_str("CardiganZip.FilePromise");
        unsafe { NSError::errorWithDomain_code_userInfo(&domain, 1, None) }
    }
}

#[cfg(target_os = "macos")]
mod macos_system_icons {
    #![allow(deprecated)]

    use super::{SystemIconRequest, SystemIconResult};
    use base64::{engine::general_purpose, Engine as _};
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSImageNameFolder, NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSSize, NSString};
    use std::path::Path;

    pub fn system_file_icons(items: Vec<SystemIconRequest>) -> Vec<SystemIconResult> {
        let workspace = NSWorkspace::sharedWorkspace();
        let generic_icon =
            icon_data_url(&workspace.iconForFileType(&NSString::from_str("public.data")));

        items
            .into_iter()
            .map(|item| {
                let data_url = icon_for_item(&workspace, &item)
                    .and_then(|image| icon_data_url(&image))
                    .filter(|data_url| {
                        item.kind == "folder"
                            || generic_icon
                                .as_ref()
                                .map(|generic| generic != data_url)
                                .unwrap_or(true)
                    });

                SystemIconResult {
                    key: item.key,
                    data_url,
                }
            })
            .collect()
    }

    fn icon_for_item(
        workspace: &NSWorkspace,
        item: &SystemIconRequest,
    ) -> Option<objc2::rc::Retained<NSImage>> {
        if item.kind == "folder" {
            if let Some(path) = item.path.as_deref().filter(|path| Path::new(path).exists()) {
                return Some(workspace.iconForFile(&NSString::from_str(path)));
            }
            return unsafe { NSImage::imageNamed(NSImageNameFolder) };
        }

        if let Some(path) = item.path.as_deref().filter(|path| Path::new(path).exists()) {
            return Some(workspace.iconForFile(&NSString::from_str(path)));
        }

        let extension = Path::new(&item.name)
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| !extension.trim().is_empty())
            .unwrap_or("public.data");
        Some(workspace.iconForFileType(&NSString::from_str(extension)))
    }

    fn icon_data_url(image: &NSImage) -> Option<String> {
        image.setSize(NSSize::new(32.0, 32.0));
        let tiff_data = image.TIFFRepresentation()?;
        let bitmap = NSBitmapImageRep::imageRepWithData(&tiff_data)?;
        let properties: objc2::rc::Retained<
            NSDictionary<objc2_app_kit::NSBitmapImageRepPropertyKey, AnyObject>,
        > = NSDictionary::new();
        let png_data = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        }?;
        Some(format!(
            "data:image/png;base64,{}",
            general_purpose::STANDARD.encode(png_data.to_vec())
        ))
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_system_icons {
    use super::{SystemIconRequest, SystemIconResult};

    pub fn system_file_icons(items: Vec<SystemIconRequest>) -> Vec<SystemIconResult> {
        items
            .into_iter()
            .map(|item| SystemIconResult {
                key: item.key,
                data_url: None,
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod macos_open_with {
    #![allow(deprecated)]

    use super::OpenWithApplication;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    use std::{
        collections::HashSet,
        path::{Path, PathBuf},
        process::Command,
    };

    pub fn suggest_open_with_apps(path: &Path) -> Vec<OpenWithApplication> {
        let workspace = NSWorkspace::sharedWorkspace();
        let file_url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let default_path = workspace
            .URLForApplicationToOpenURL(&file_url)
            .and_then(|url| url.path())
            .map(|path| path.to_string());
        let mut seen = HashSet::new();
        let mut apps = Vec::new();

        if let Some(path) = default_path.as_deref() {
            push_application(&mut apps, &mut seen, path, true);
        }

        let app_urls = workspace.URLsForApplicationsToOpenURL(&file_url);
        for index in 0..app_urls.count() {
            if let Some(path) = app_urls.objectAtIndex(index).path() {
                let path = path.to_string();
                let is_default = default_path.as_deref() == Some(path.as_str());
                push_application(&mut apps, &mut seen, &path, is_default);
            }
        }

        apps
    }

    pub fn open_file_with_application(
        path: &Path,
        application_path: Option<&Path>,
    ) -> Result<(), String> {
        let mut command = Command::new("/usr/bin/open");
        if let Some(application_path) = application_path {
            command.arg("-a").arg(application_path);
        }
        command.arg(path);

        let output = command
            .output()
            .map_err(|err| format!("无法打开文件：{err}"))?;
        if output.status.success() {
            return Ok(());
        }

        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if message.is_empty() {
            "无法用所选应用打开文件。".to_string()
        } else {
            message
        })
    }

    fn push_application(
        apps: &mut Vec<OpenWithApplication>,
        seen: &mut HashSet<String>,
        path: &str,
        is_default: bool,
    ) {
        let clean_path = path.trim();
        if clean_path.is_empty() || !seen.insert(clean_path.to_string()) {
            return;
        }
        apps.push(OpenWithApplication {
            name: application_name(clean_path),
            path: clean_path.to_string(),
            is_default,
        });
    }

    fn application_name(path: &str) -> String {
        let path = PathBuf::from(path);
        path.file_stem()
            .or_else(|| path.file_name())
            .and_then(|name| name.to_str())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or("应用程序")
            .to_string()
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_open_with {
    use super::OpenWithApplication;
    use std::path::Path;

    pub fn suggest_open_with_apps(_path: &Path) -> Vec<OpenWithApplication> {
        Vec::new()
    }

    pub fn open_file_with_application(
        _path: &Path,
        _application_path: Option<&Path>,
    ) -> Result<(), String> {
        Err("当前平台暂不支持选择打开方式。".to_string())
    }
}

#[cfg(target_os = "macos")]
mod macos_finder_services {
    #![allow(deprecated)]

    use super::{
        extract_archives_from_finder_service, route_system_open_paths, FinderExtractDestination,
    };
    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, NSObjectProtocol},
        AnyThread, ClassType, DefinedClass,
    };
    use objc2_app_kit::{NSApp, NSFilenamesPboardType, NSPasteboard};
    use objc2_foundation::{NSArray, NSString, NSURL};
    use std::path::PathBuf;

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = objc2::AnyThread]
        #[name = "PackoFinderServiceProvider"]
        #[ivars = FinderServiceProviderIvars]
        struct FinderServiceProvider;

        unsafe impl NSObjectProtocol for FinderServiceProvider {}

        impl FinderServiceProvider {
            #[unsafe(method(packoCompressSelection:userData:error:))]
            unsafe fn packo_compress_selection(
                &self,
                pboard: &NSPasteboard,
                _user_data: &NSString,
                _error: *mut *mut NSString,
            ) {
                let paths = service_paths_from_pasteboard(pboard);
                if paths.is_empty() {
                    return;
                }
                if let Err(error) = route_system_open_paths(&self.ivars().app, paths) {
                    eprintln!("Packo Finder compress service failed: {error}");
                }
            }

            #[unsafe(method(packoExtractHere:userData:error:))]
            unsafe fn packo_extract_here(
                &self,
                pboard: &NSPasteboard,
                _user_data: &NSString,
                _error: *mut *mut NSString,
            ) {
                let paths = service_paths_from_pasteboard(pboard);
                if paths.is_empty() {
                    return;
                }
                extract_archives_from_finder_service(
                    paths,
                    FinderExtractDestination::CurrentDirectory,
                );
            }

            #[unsafe(method(packoExtractToFolder:userData:error:))]
            unsafe fn packo_extract_to_folder(
                &self,
                pboard: &NSPasteboard,
                _user_data: &NSString,
                _error: *mut *mut NSString,
            ) {
                let paths = service_paths_from_pasteboard(pboard);
                if paths.is_empty() {
                    return;
                }
                extract_archives_from_finder_service(
                    paths,
                    FinderExtractDestination::ArchiveNamedFolder,
                );
            }
        }
    );

    struct FinderServiceProviderIvars {
        app: tauri::AppHandle<tauri::Wry>,
    }

    impl FinderServiceProvider {
        fn new(app: tauri::AppHandle<tauri::Wry>) -> Retained<Self> {
            let this = Self::alloc().set_ivars(FinderServiceProviderIvars { app });
            unsafe { msg_send![super(this), init] }
        }
    }

    pub fn register(app: tauri::AppHandle<tauri::Wry>) {
        unsafe {
            let provider = FinderServiceProvider::new(app);
            let provider_object: Retained<AnyObject> = provider.into_super().into_super();
            NSApp(objc2::MainThreadMarker::new_unchecked())
                .setServicesProvider(Some(&provider_object));
            std::mem::forget(provider_object);
        }
    }

    fn service_paths_from_pasteboard(pboard: &NSPasteboard) -> Vec<PathBuf> {
        let mut paths = file_urls_from_pasteboard(pboard);
        if paths.is_empty() {
            paths = legacy_file_names_from_pasteboard(pboard);
        }
        paths
    }

    fn file_urls_from_pasteboard(pboard: &NSPasteboard) -> Vec<PathBuf> {
        unsafe {
            let classes = NSArray::from_slice(&[NSURL::class()]);
            let Some(objects) = pboard.readObjectsForClasses_options(&classes, None) else {
                return Vec::new();
            };

            let mut paths = Vec::new();
            for index in 0..objects.count() {
                let object = objects.objectAtIndex(index);
                let is_file_url: bool = msg_send![&*object, isFileURL];
                if !is_file_url {
                    continue;
                }
                let path: Option<Retained<NSString>> = msg_send![&*object, path];
                if let Some(path) = path {
                    let value = path.to_string();
                    if !value.trim().is_empty() {
                        paths.push(PathBuf::from(value));
                    }
                }
            }
            paths
        }
    }

    fn legacy_file_names_from_pasteboard(pboard: &NSPasteboard) -> Vec<PathBuf> {
        unsafe {
            let Some(object) = pboard.propertyListForType(NSFilenamesPboardType) else {
                return Vec::new();
            };

            let count: usize = msg_send![&*object, count];
            let mut paths = Vec::new();
            for index in 0..count {
                let path: Retained<NSString> = msg_send![&*object, objectAtIndex: index];
                let value = path.to_string();
                if !value.trim().is_empty() {
                    paths.push(PathBuf::from(value));
                }
            }
            paths
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod macos_finder_services {
    pub fn register(_app: tauri::AppHandle<tauri::Wry>) {}
}

#[derive(Debug, Serialize)]
struct FileInfo {
    path: String,
    name: String,
    kind: String,
    size: u64,
    size_label: String,
    modified_label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SystemIconRequest {
    key: String,
    path: Option<String>,
    name: String,
    kind: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemIconResult {
    key: String,
    data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenWithApplication {
    name: String,
    path: String,
    is_default: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultArchiveOpenerResult {
    updated: usize,
    failed: Vec<DefaultArchiveOpenerFailure>,
    registered_app_path: Option<String>,
    warning: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultArchiveOpenerFailure {
    content_type: String,
    status: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchivePromiseDragItem {
    entry_path: String,
    promised_name: String,
    is_dir: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchivePromiseDragProgress {
    drag_id: String,
    status: String,
    total: usize,
    completed: usize,
    current_item: String,
    message: String,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ArchiveEntry {
    name: String,
    path: String,
    kind: String,
    type_label: String,
    size: u64,
    size_label: String,
    modified_label: String,
    crc: Option<String>,
    method: Option<String>,
    is_encrypted: bool,
    is_hidden: bool,
    is_executable: bool,
    is_unsafe_path: bool,
}

#[derive(Debug, Serialize)]
struct ArchiveSplitSummary {
    is_split: bool,
    volume_count: usize,
    total_size: u64,
    total_size_label: String,
    first_volume: String,
}

#[derive(Debug, Serialize)]
struct ArchiveProperties {
    uncompressed_size: u64,
    uncompressed_size_label: String,
    compression_ratio_label: String,
    method_summary: String,
    crc_available: bool,
    is_encrypted: bool,
    encrypted_count: usize,
    split: ArchiveSplitSummary,
}

#[derive(Debug, Serialize)]
struct ArchiveSafetySummary {
    unsafe_paths: usize,
    hidden_files: usize,
    executables: usize,
    samples: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ArchiveInfo {
    path: String,
    name: String,
    format: String,
    size: u64,
    size_label: String,
    file_count: usize,
    folder_count: usize,
    created_label: String,
    output_dir: String,
    entries: Vec<ArchiveEntry>,
    folders: Vec<String>,
    safety: ArchiveSafetySummary,
    properties: ArchiveProperties,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressOptions {
    source_paths: Vec<String>,
    output_dir: String,
    archive_name: String,
    format: String,
    compression_level: Option<u8>,
    password: Option<String>,
    volume_size_mb: Option<u64>,
    #[serde(default)]
    excluded_paths: Vec<String>,
    #[serde(default)]
    skip_ds_store: bool,
    #[serde(default)]
    advanced: CompressAdvancedOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompressAdvancedOptions {
    #[serde(default)]
    batch_queue: bool,
    dictionary_size_mb: Option<u32>,
    solid: Option<bool>,
    threads: Option<u32>,
    method: Option<String>,
    #[serde(default = "default_test_after_compress")]
    test_after_compress: bool,
    #[serde(default)]
    skip_macos_metadata: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveRenameEntry {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveReplaceEntry {
    entry_path: String,
    source_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveAddEntry {
    source_path: String,
    target_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveEditOptions {
    path: String,
    password: Option<String>,
    #[serde(default)]
    delete_entries: Vec<String>,
    #[serde(default)]
    rename_entries: Vec<ArchiveRenameEntry>,
    #[serde(default)]
    add_paths: Vec<String>,
    #[serde(default)]
    add_entries: Vec<ArchiveAddEntry>,
    #[serde(default)]
    create_dirs: Vec<String>,
    #[serde(default)]
    replace_entries: Vec<ArchiveReplaceEntry>,
    output_path: Option<String>,
}

impl Default for CompressAdvancedOptions {
    fn default() -> Self {
        Self {
            batch_queue: false,
            dictionary_size_mb: None,
            solid: None,
            threads: None,
            method: None,
            test_after_compress: true,
            skip_macos_metadata: false,
        }
    }
}

fn default_test_after_compress() -> bool {
    true
}

#[derive(Debug, Serialize)]
struct OperationResult {
    output_path: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct PreviewResult {
    output_path: String,
    message: String,
}

#[derive(Default)]
struct ExtractTasks {
    tasks: Mutex<HashMap<String, Arc<ExtractTask>>>,
}

#[derive(Default)]
struct CompressTasks {
    tasks: Mutex<HashMap<String, Arc<ExtractTask>>>,
}

#[derive(Default)]
struct WorkWindowPayloads {
    payloads: Mutex<HashMap<String, WorkWindowPayload>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
enum WorkWindowPayload {
    Extract { paths: Vec<String> },
    Compress { paths: Vec<String> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinderExtractDestination {
    CurrentDirectory,
    ArchiveNamedFolder,
}

struct ExtractTask {
    state: Mutex<ExtractTaskState>,
}

struct ExtractTaskState {
    status: String,
    total: usize,
    completed: usize,
    total_bytes: u64,
    completed_bytes: u64,
    current_bytes: u64,
    current_total_bytes: u64,
    current_item: String,
    output_path: String,
    message: String,
    error: Option<String>,
    cancel_requested: bool,
    pause_requested: bool,
    child_pid: Option<u32>,
}

#[derive(Debug, Serialize, Clone)]
struct ExtractTaskProgress {
    task_id: String,
    status: String,
    total: usize,
    completed: usize,
    total_bytes: u64,
    completed_bytes: u64,
    current_bytes: u64,
    current_total_bytes: u64,
    current_item: String,
    output_path: String,
    message: String,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ExtractTaskEntry {
    path: String,
    display_path: String,
    size: u64,
    is_dir: bool,
    is_unsafe_path: bool,
}

#[derive(Debug, Clone)]
struct BatchExtractPlan {
    archive_path: PathBuf,
    output_dir: PathBuf,
    entries: Vec<ExtractTaskEntry>,
}

enum ExtractTaskError {
    Canceled,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExtractConflictStrategy {
    Overwrite,
    Skip,
    Rename,
}

fn extract_task_error_message(error: ExtractTaskError) -> String {
    match error {
        ExtractTaskError::Canceled => "操作已取消。".to_string(),
        ExtractTaskError::Failed(message) => message,
    }
}

struct CompressPlan {
    runner: CompressRunner,
    output_path: PathBuf,
    work_output_path: PathBuf,
    entries: Vec<ExtractTaskEntry>,
    total_bytes: u64,
    format: String,
    password: Option<String>,
    volume_size_mb: Option<u64>,
    test_after_compress: bool,
}

enum CompressRunner {
    Command {
        program: &'static str,
        args: Vec<OsString>,
        current_dir: Option<PathBuf>,
        disable_macos_metadata: bool,
    },
    SevenZip {
        common_parent: PathBuf,
        source_paths: Vec<PathBuf>,
        settings: SevenZipCompressSettings,
    },
}

#[derive(Debug, Clone)]
struct SevenZipCompressSettings {
    password: Option<String>,
    compression_level: u8,
    dictionary_size_mb: Option<u32>,
    threads: u32,
    solid: bool,
    method: SevenZipMethod,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum SevenZipMethod {
    Lzma2,
    Lzma,
}

#[tauri::command]
fn describe_paths(paths: Vec<String>) -> Result<Vec<FileInfo>, String> {
    paths
        .iter()
        .map(|path| describe_path(Path::new(path)))
        .collect()
}

#[tauri::command]
fn system_file_icons(items: Vec<SystemIconRequest>) -> Vec<SystemIconResult> {
    macos_system_icons::system_file_icons(items)
}

#[tauri::command]
fn open_full_disk_access_settings() -> Result<(), String> {
    open_macos_full_disk_access_settings()
}

#[tauri::command]
fn reveal_packo_app_in_finder() -> Result<String, String> {
    reveal_packo_app_in_finder_impl()
}

#[tauri::command]
fn set_packo_as_default_archive_opener() -> Result<DefaultArchiveOpenerResult, String> {
    set_packo_as_default_archive_opener_impl()
}

#[cfg(target_os = "macos")]
fn open_macos_full_disk_access_settings() -> Result<(), String> {
    let targets = [
        "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles",
        "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        "/System/Applications/System Settings.app",
        "/System/Applications/System Preferences.app",
    ];
    let mut errors = Vec::new();

    for target in targets {
        match Command::new("open").arg(target).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => errors.push(format!("{target} 退出状态 {status}")),
            Err(error) => errors.push(format!("{target}: {error}")),
        }
    }

    Err(format!("无法自动打开系统设置：{}", errors.join("；")))
}

#[cfg(not(target_os = "macos"))]
fn open_macos_full_disk_access_settings() -> Result<(), String> {
    Err("完全磁盘访问仅适用于 macOS。".to_string())
}

#[cfg(target_os = "macos")]
fn reveal_packo_app_in_finder_impl() -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|err| format!("无法定位 Packo：{err}"))?;
    let target = macos_app_bundle_path(&executable).unwrap_or(executable);
    let status = Command::new("open")
        .arg("-R")
        .arg(&target)
        .status()
        .map_err(|err| format!("无法打开 Finder：{err}"))?;
    if !status.success() {
        return Err(format!("Finder 返回失败状态：{status}"));
    }
    Ok(target.to_string_lossy().to_string())
}

#[cfg(target_os = "macos")]
fn macos_app_bundle_path(path: &Path) -> Option<PathBuf> {
    path.ancestors().find_map(|ancestor| {
        (ancestor
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("app"))
        .then(|| ancestor.to_path_buf())
    })
}

#[cfg(not(target_os = "macos"))]
fn reveal_packo_app_in_finder_impl() -> Result<String, String> {
    Err("仅 macOS 支持在 Finder 中定位 Packo。".to_string())
}

#[cfg(target_os = "macos")]
const PACKO_BUNDLE_IDENTIFIER: &str = "com.packo.desktop";

#[cfg(target_os = "macos")]
const PACKO_ARCHIVE_DEFAULT_CONTENT_TYPES: &[&str] = &[
    "public.zip-archive",
    "com.pkware.zip-archive",
    "org.7-zip.7-zip-archive",
    "com.rarlab.rar-archive",
    "public.tar-archive",
    "org.gnu.gnu-tar-archive",
    "org.gnu.gnu-zip-archive",
    "org.gnu.gnu-zip-tar-archive",
    "public.bzip2-archive",
    "public.tar-bzip2-archive",
    "org.bzip.bzip2-archive",
    "org.bzip.bzip2-tar-archive",
    "org.tukaani.xz-archive",
    "org.tukaani.tar-xz-archive",
    "public.xz-archive",
    "public.tar-xz-archive",
    "public.z-archive",
    "com.facebook.zstandard-archive",
    "com.facebook.zstandard-tar-archive",
    "org.tukaani.lzma-archive",
    "public.lzma-archive",
    "public.lz4-archive",
    "public.lz4-tar-archive",
    "public.iso-image",
    "public.archive.lha",
    "com.bandisoft.001",
];

#[cfg(target_os = "macos")]
const PACKO_ARCHIVE_DEFAULT_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "tar", "gz", "gzip", "bz2", "bzip2", "xz", "tgz", "tbz", "tbz2", "txz",
    "lzh", "lha", "z", "zst", "zstd", "lzma", "lzma2", "lz4", "iso", "001",
];

#[cfg(target_os = "macos")]
const K_LS_ROLES_VIEWER: u32 = 0x00000002;

#[cfg(target_os = "macos")]
const LSREGISTER_PATH: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";

#[cfg(target_os = "macos")]
#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSSetDefaultRoleHandlerForContentType(
        in_content_type: core_foundation::string::CFStringRef,
        in_role: u32,
        in_handler_bundle_id: core_foundation::string::CFStringRef,
    ) -> i32;

    fn UTTypeCreatePreferredIdentifierForTag(
        in_tag_class: core_foundation::string::CFStringRef,
        in_tag: core_foundation::string::CFStringRef,
        in_conforming_to_uti: core_foundation::string::CFStringRef,
    ) -> core_foundation::string::CFStringRef;
}

#[cfg(target_os = "macos")]
fn set_packo_as_default_archive_opener_impl() -> Result<DefaultArchiveOpenerResult, String> {
    use core_foundation::{base::TCFType, string::CFString};

    let (registered_app_path, warning) = match register_packo_app_bundle_for_launch_services() {
        Ok(path) => (path, None),
        Err(error) => (None, Some(error)),
    };

    let handler_bundle_id = CFString::new(PACKO_BUNDLE_IDENTIFIER);
    let mut updated = 0;
    let mut failed = Vec::new();

    for content_type in packo_archive_default_content_types() {
        let content_type_ref = CFString::new(&content_type);
        let status = unsafe {
            LSSetDefaultRoleHandlerForContentType(
                content_type_ref.as_concrete_TypeRef(),
                K_LS_ROLES_VIEWER,
                handler_bundle_id.as_concrete_TypeRef(),
            )
        };

        if status == 0 {
            updated += 1;
        } else {
            failed.push(DefaultArchiveOpenerFailure {
                content_type,
                status,
            });
        }
    }

    if updated == 0 {
        let failed_summary = failed
            .iter()
            .take(6)
            .map(|failure| format!("{}({})", failure.content_type, failure.status))
            .collect::<Vec<_>>()
            .join("、");
        let warning_suffix = warning
            .as_ref()
            .map(|message| format!("；{message}"))
            .unwrap_or_default();
        return Err(format!(
            "无法把 Packo 设为压缩包默认打开方式：{failed_summary}{warning_suffix}"
        ));
    }

    Ok(DefaultArchiveOpenerResult {
        updated,
        failed,
        registered_app_path,
        warning,
    })
}

#[cfg(not(target_os = "macos"))]
fn set_packo_as_default_archive_opener_impl() -> Result<DefaultArchiveOpenerResult, String> {
    Err("默认打开方式设置仅适用于 macOS。".to_string())
}

#[cfg(target_os = "macos")]
fn packo_archive_default_content_types() -> BTreeSet<String> {
    let mut content_types = PACKO_ARCHIVE_DEFAULT_CONTENT_TYPES
        .iter()
        .map(|content_type| (*content_type).to_string())
        .collect::<BTreeSet<_>>();

    for extension in PACKO_ARCHIVE_DEFAULT_EXTENSIONS {
        if let Some(content_type) = preferred_content_type_for_extension(extension) {
            content_types.insert(content_type);
        }
    }

    for index in 1..=99 {
        let extension = format!("z{index:02}");
        if let Some(content_type) = preferred_content_type_for_extension(&extension) {
            content_types.insert(content_type);
        }
    }

    content_types
}

#[cfg(target_os = "macos")]
fn preferred_content_type_for_extension(extension: &str) -> Option<String> {
    use core_foundation::{base::TCFType, string::CFString};

    let tag_class = CFString::new("public.filename-extension");
    let tag = CFString::new(extension);
    let content_type_ref = unsafe {
        UTTypeCreatePreferredIdentifierForTag(
            tag_class.as_concrete_TypeRef(),
            tag.as_concrete_TypeRef(),
            std::ptr::null(),
        )
    };

    if content_type_ref.is_null() {
        return None;
    }

    let content_type = unsafe { CFString::wrap_under_create_rule(content_type_ref) };
    let content_type = content_type.to_string();
    (!content_type.trim().is_empty()).then_some(content_type)
}

#[cfg(target_os = "macos")]
fn register_packo_app_bundle_for_launch_services() -> Result<Option<String>, String> {
    let mut errors = Vec::new();

    for app_path in packo_app_bundle_candidates() {
        if !app_path.exists() {
            continue;
        }

        match Command::new(LSREGISTER_PATH)
            .arg("-f")
            .arg(&app_path)
            .status()
        {
            Ok(status) if status.success() => {
                return Ok(Some(app_path.to_string_lossy().to_string()));
            }
            Ok(status) => errors.push(format!(
                "{} 注册失败，退出状态 {status}",
                app_path.display()
            )),
            Err(error) => errors.push(format!("{} 注册失败：{error}", app_path.display())),
        }
    }

    if errors.is_empty() {
        Ok(None)
    } else {
        Err(format!(
            "Packo.app 注册到系统默认打开方式数据库时失败：{}",
            errors.join("；")
        ))
    }
}

#[cfg(target_os = "macos")]
fn packo_app_bundle_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(executable) = std::env::current_exe() {
        if let Some(app_path) = macos_app_bundle_path(&executable) {
            candidates.push(app_path);
        }
    }

    candidates.push(PathBuf::from("/Applications/Packo.app"));

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("src-tauri/target/release/bundle/macos/Packo.app"));
        candidates.push(current_dir.join("target/release/bundle/macos/Packo.app"));
        candidates.push(current_dir.join("target/debug/bundle/macos/Packo.app"));
    }

    let mut seen = BTreeSet::new();
    candidates
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[tauri::command]
fn describe_compress_paths(
    paths: Vec<String>,
    excluded_paths: Vec<String>,
    skip_ds_store: Option<bool>,
    skip_macos_metadata: Option<bool>,
) -> Result<Vec<FileInfo>, String> {
    let excluded_paths: Vec<PathBuf> = excluded_paths.into_iter().map(PathBuf::from).collect();
    let skip_ds_store = skip_ds_store.unwrap_or(false);
    let skip_macos_metadata = skip_macos_metadata.unwrap_or(false);
    let mut entries = Vec::new();

    for path in paths {
        collect_compress_file_info(
            Path::new(&path),
            &excluded_paths,
            skip_ds_store,
            skip_macos_metadata,
            &mut entries,
        )?;
    }

    Ok(entries)
}

fn normalize_archive_password(password: Option<String>) -> Option<String> {
    password
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_extract_conflict_strategy(
    strategy: Option<String>,
) -> Result<ExtractConflictStrategy, String> {
    match strategy
        .unwrap_or_else(|| "overwrite".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "overwrite" | "cover" | "replace" => Ok(ExtractConflictStrategy::Overwrite),
        "skip" => Ok(ExtractConflictStrategy::Skip),
        "rename" | "auto_rename" | "auto-rename" => Ok(ExtractConflictStrategy::Rename),
        _ => Err("未知的解压冲突处理方式。".to_string()),
    }
}

const ARCHIVE_PASSWORD_PROBE: &str = "__PACKO_PASSWORD_REQUIRED__";

struct PreparedArchive {
    path: PathBuf,
    temp_dir: Option<PathBuf>,
}

impl PreparedArchive {
    fn borrowed(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            temp_dir: None,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PreparedArchive {
    fn drop(&mut self) {
        if let Some(temp_dir) = self.temp_dir.take() {
            let _ = fs::remove_dir_all(temp_dir);
        }
    }
}

fn append_archive_password_args(args: &mut Vec<OsString>, password: Option<&str>) {
    if let Some(password) = password.map(str::trim).filter(|value| !value.is_empty()) {
        args.push(OsString::from("--passphrase"));
        args.push(OsString::from(password));
    }
}

fn append_archive_extract_password_args(args: &mut Vec<OsString>, password: Option<&str>) {
    args.push(OsString::from("--passphrase"));
    args.push(OsString::from(
        password
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(ARCHIVE_PASSWORD_PROBE),
    ));
}

#[tauri::command]
fn list_archive(path: String, password: Option<String>) -> Result<ArchiveInfo, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
    let password = normalize_archive_password(password);
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;

    let prepared_archive = if is_single_stream_format(&archive_path) {
        None
    } else {
        Some(prepare_archive_for_read(&archive_path)?)
    };
    let read_archive_path = prepared_archive
        .as_ref()
        .map(|archive| archive.path())
        .unwrap_or(archive_path.as_path());

    let mut entries = if is_single_stream_format(&archive_path) {
        vec![single_stream_entry(&archive_path)?]
    } else {
        read_archive_listing_entries(read_archive_path, password.as_deref())?
    };
    apply_archive_entry_details(read_archive_path, &mut entries);
    let archive_is_encrypted =
        detect_archive_encryption(read_archive_path, &entries, password.as_deref())
            .unwrap_or(false);
    if archive_is_encrypted {
        for entry in &mut entries {
            if entry.kind != "folder" {
                entry.is_encrypted = true;
            }
        }
    }
    let folders = collect_folders(&entries);
    let file_count = entries
        .iter()
        .filter(|entry| entry.kind != "folder")
        .count();
    let folder_count = entries
        .iter()
        .filter(|entry| entry.kind == "folder")
        .count();
    let safety = archive_safety_summary(&entries);
    let metadata = fs::metadata(&archive_path).map_err(|err| err.to_string())?;
    let properties = archive_properties(
        &archive_path,
        &entries,
        metadata.len(),
        archive_is_encrypted,
    );
    let parent = archive_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_string_lossy()
        .to_string();

    Ok(ArchiveInfo {
        path: archive_path.to_string_lossy().to_string(),
        name: file_name(&archive_path),
        format: archive_format(&archive_path),
        size: metadata.len(),
        size_label: format_size(metadata.len()),
        file_count,
        folder_count,
        created_label: metadata
            .created()
            .or_else(|_| metadata.modified())
            .ok()
            .map(format_system_time)
            .unwrap_or_else(|| "-".to_string()),
        output_dir: parent,
        entries,
        folders,
        safety,
        properties,
    })
}

#[tauri::command]
fn test_archive_integrity(
    path: String,
    password: Option<String>,
) -> Result<OperationResult, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
    let password = normalize_archive_password(password);
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;
    test_archive_integrity_path(&archive_path, password.as_deref())?;

    Ok(OperationResult {
        output_path: archive_path.to_string_lossy().to_string(),
        message: "压缩包完整性测试通过。".to_string(),
    })
}

#[tauri::command]
fn clear_preview_cache(path: Option<String>) -> Result<OperationResult, String> {
    let output_path = if let Some(path) = path.filter(|value| !value.trim().is_empty()) {
        let archive_path = PathBuf::from(path);
        let preview_dir = preview_output_dir(&archive_path)?;
        if preview_dir.exists() {
            fs::remove_dir_all(&preview_dir).map_err(|err| format!("无法清理预览缓存：{err}"))?;
        }
        preview_dir
    } else {
        let preview_root = std::env::temp_dir().join("cardiganzip-preview");
        if preview_root.exists() {
            fs::remove_dir_all(&preview_root).map_err(|err| format!("无法清理预览缓存：{err}"))?;
        }
        preview_root
    };

    Ok(OperationResult {
        output_path: output_path.to_string_lossy().to_string(),
        message: "预览缓存已清理。".to_string(),
    })
}

#[tauri::command]
fn edit_archive(options: ArchiveEditOptions) -> Result<OperationResult, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(options.path.trim()))?;
    let password = normalize_archive_password(options.password);
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;
    ensure_archive_is_editable(&archive_path)?;

    let format = normalize_edit_archive_format(&archive_path, options.output_path.as_deref())?;
    let temp_dir = archive_edit_temp_dir(&archive_path);
    let content_dir = temp_dir.join("content");
    let build_dir = temp_dir.join("build");
    let result = (|| -> Result<OperationResult, String> {
        fs::create_dir_all(&content_dir).map_err(|err| format!("无法创建编辑临时目录：{err}"))?;
        fs::create_dir_all(&build_dir).map_err(|err| format!("无法创建编辑输出目录：{err}"))?;

        let task_entries = extract_task_entries(&archive_path, Vec::new(), password.as_deref())?;
        ensure_safe_archive_entry_paths(&task_entries)?;
        extract_entries_to_dir(&archive_path, &content_dir, &[], password.as_deref())?;

        apply_archive_deletes(&content_dir, &options.delete_entries)?;
        apply_archive_renames(&content_dir, &options.rename_entries)?;
        apply_archive_create_dirs(&content_dir, &options.create_dirs)?;
        apply_archive_replacements(&content_dir, &options.replace_entries)?;
        apply_archive_additions(&content_dir, &options.add_paths)?;
        apply_archive_add_entries(&content_dir, &options.add_entries)?;

        let source_paths = archive_edit_source_paths(&content_dir)?;
        if source_paths.is_empty() {
            return Err("压缩包不能为空。".to_string());
        }

        let rebuilt = compress_archive(CompressOptions {
            source_paths,
            output_dir: build_dir.to_string_lossy().to_string(),
            archive_name: "packo-edit".to_string(),
            format: format.clone(),
            compression_level: None,
            password: password
                .clone()
                .filter(|_| supports_compress_password(&format)),
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions {
                skip_macos_metadata: true,
                ..CompressAdvancedOptions::default()
            },
        })?;

        let rebuilt_path = PathBuf::from(rebuilt.output_path);
        let output_path = options
            .output_path
            .as_deref()
            .map(|path| PathBuf::from(path.trim()))
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| archive_path.clone());
        install_rebuilt_archive(&rebuilt_path, &output_path)?;
        Ok(OperationResult {
            output_path: output_path.to_string_lossy().to_string(),
            message: "压缩包编辑已保存。".to_string(),
        })
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

#[tauri::command]
fn compress_archive(options: CompressOptions) -> Result<OperationResult, String> {
    let plans = prepare_compress_plans(&options)?;
    for plan in &plans {
        run_compress_plan(plan)?;
    }
    let output_path = if plans.len() == 1 {
        plans[0].output_path.to_string_lossy().to_string()
    } else {
        options.output_dir.trim().to_string()
    };

    Ok(OperationResult {
        output_path,
        message: "压缩完成。".to_string(),
    })
}

#[tauri::command]
fn start_compress_task(
    tasks: tauri::State<'_, CompressTasks>,
    options: CompressOptions,
) -> Result<ExtractTaskProgress, String> {
    let plans = prepare_compress_plans(&options)?;
    let total = plans
        .iter()
        .map(|plan| plan.entries.len().max(1))
        .sum::<usize>()
        .max(1);
    let total_bytes = plans
        .iter()
        .map(|plan| plan.total_bytes.max(1))
        .sum::<u64>()
        .max(1);
    let output_path = if plans.len() == 1 {
        plans[0].output_path.clone()
    } else {
        PathBuf::from(options.output_dir.trim())
    };
    let task_id = format!(
        "compress-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let task = Arc::new(ExtractTask::new(
        total,
        total_bytes,
        output_path.to_string_lossy().to_string(),
    ));

    {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法创建压缩任务。".to_string())?;
        state.current_item = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("压缩包")
            .to_string();
        state.current_total_bytes = total_bytes;
        state.message = "正在创建压缩任务。".to_string();
    }

    tasks
        .tasks
        .lock()
        .map_err(|_| "无法创建压缩任务。".to_string())?
        .insert(task_id.clone(), Arc::clone(&task));

    let thread_task_id = task_id.clone();
    thread::spawn(move || {
        run_compress_task(thread_task_id, task, plans);
    });

    get_compress_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn get_compress_task(
    tasks: tauri::State<'_, CompressTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    get_compress_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn pause_compress_task(
    tasks: tauri::State<'_, CompressTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    let task = find_compress_task(&tasks, &task_id)?;
    let child_pid = {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法暂停压缩任务。".to_string())?;
        if state.status == "completed" || state.status == "failed" || state.status == "canceled" {
            return Ok(extract_task_progress(&task_id, &state));
        }
        state.pause_requested = true;
        state.status = "paused".to_string();
        state.message = "压缩已暂停。".to_string();
        state.child_pid
    };

    if let Some(pid) = child_pid {
        let _ = send_process_signal(pid, "STOP");
    }

    get_compress_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn resume_compress_task(
    tasks: tauri::State<'_, CompressTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    let task = find_compress_task(&tasks, &task_id)?;
    let child_pid = {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法继续压缩任务。".to_string())?;
        if state.status == "completed" || state.status == "failed" || state.status == "canceled" {
            return Ok(extract_task_progress(&task_id, &state));
        }
        state.pause_requested = false;
        state.status = "running".to_string();
        state.message = "正在继续压缩。".to_string();
        state.child_pid
    };

    if let Some(pid) = child_pid {
        let _ = send_process_signal(pid, "CONT");
    }

    get_compress_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn cancel_compress_task(
    tasks: tauri::State<'_, CompressTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    let task = find_compress_task(&tasks, &task_id)?;
    let child_pid = {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法取消压缩任务。".to_string())?;
        if state.status == "completed" || state.status == "failed" || state.status == "canceled" {
            return Ok(extract_task_progress(&task_id, &state));
        }
        state.cancel_requested = true;
        state.pause_requested = false;
        state.status = "canceling".to_string();
        state.message = "正在取消压缩。".to_string();
        state.child_pid
    };

    if let Some(pid) = child_pid {
        let _ = send_process_signal(pid, "CONT");
        let _ = send_process_signal(pid, "TERM");
    }

    get_compress_task_progress(&tasks, &task_id)
}

fn run_compress_plan(plan: &CompressPlan) -> Result<(), String> {
    run_compress_plan_body(plan).map_err(extract_task_error_message)?;
    finalize_compress_plan(plan, None).map_err(extract_task_error_message)
}

fn run_compress_plan_body(plan: &CompressPlan) -> Result<(), ExtractTaskError> {
    match &plan.runner {
        CompressRunner::Command {
            program,
            args,
            current_dir,
            disable_macos_metadata,
        } => {
            run_command_with_env(
                program,
                args.iter().map(|arg| arg.as_os_str()),
                current_dir.as_deref(),
                *disable_macos_metadata,
            )
            .map_err(ExtractTaskError::Failed)?;
            Ok(())
        }
        CompressRunner::SevenZip {
            common_parent,
            source_paths,
            settings,
        } => run_seven_zip_compress_plan(plan, common_parent, source_paths, settings, None),
    }
}

fn finalize_compress_plan(
    plan: &CompressPlan,
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    if plan.test_after_compress {
        if let Some((_, task)) = progress {
            wait_for_extract_task(task)?;
            let mut state = task_state(task)?;
            state.status = "running".to_string();
            state.current_item = file_name(&plan.work_output_path);
            state.current_bytes = 0;
            state.current_total_bytes = plan.total_bytes.max(1);
            state.message = "正在测试压缩包完整性。".to_string();
        }
        test_compress_output(plan)?;
    }

    if plan.format == "7Z" && plan.volume_size_mb.is_some() {
        if let Some((_, task)) = progress {
            wait_for_extract_task(task)?;
            let mut state = task_state(task)?;
            state.status = "running".to_string();
            state.current_item = file_name(&plan.output_path);
            state.current_bytes = 0;
            state.current_total_bytes = compress_output_size(&plan.work_output_path).unwrap_or(1);
            state.message = "正在写入 7Z 分卷。".to_string();
        }

        split_file_to_volumes(
            &plan.work_output_path,
            &plan.output_path,
            plan.volume_size_mb.unwrap_or(1),
        )?;
        let _ = fs::remove_file(&plan.work_output_path);
    }

    Ok(())
}

fn test_compress_output(plan: &CompressPlan) -> Result<(), ExtractTaskError> {
    match plan.format.as_str() {
        "ZIP" => test_zip_compress_output(plan),
        "7Z" => {
            use sevenz_rust2::{ArchiveReader, Password as SevenZipPassword};

            let password = plan
                .password
                .as_deref()
                .map(SevenZipPassword::new)
                .unwrap_or_else(SevenZipPassword::empty);
            ArchiveReader::open(&plan.work_output_path, password)
                .map(|_| ())
                .map_err(|error| ExtractTaskError::Failed(format!("7Z 完整性测试失败：{error}")))
        }
        _ => {
            let mut args = Vec::new();
            append_archive_password_args(&mut args, plan.password.as_deref());
            args.push(OsString::from("-tf"));
            args.push(plan.work_output_path.as_os_str().to_os_string());
            run_command("bsdtar", args.iter().map(|arg| arg.as_os_str()), None)
                .map(|_| ())
                .map_err(|error| ExtractTaskError::Failed(format!("压缩包完整性测试失败：{error}")))
        }
    }
}

fn test_zip_compress_output(plan: &CompressPlan) -> Result<(), ExtractTaskError> {
    if plan.volume_size_mb.is_none() {
        return run_unzip_test(&plan.work_output_path, plan.password.as_deref());
    }

    let args = vec![
        OsString::from("-sf"),
        plan.work_output_path.as_os_str().to_os_string(),
    ];
    run_command("zip", args.iter().map(|arg| arg.as_os_str()), None)
        .map(|_| ())
        .map_err(|error| ExtractTaskError::Failed(format!("ZIP 分卷结构测试失败：{error}")))
}

fn run_unzip_test(path: &Path, password: Option<&str>) -> Result<(), ExtractTaskError> {
    let mut args = vec![OsString::from("-t")];
    if let Some(password) = password {
        args.push(OsString::from("-P"));
        args.push(OsString::from(password));
    }
    args.push(path.as_os_str().to_os_string());
    run_command("unzip", args.iter().map(|arg| arg.as_os_str()), None)
        .map(|_| ())
        .map_err(|error| ExtractTaskError::Failed(format!("ZIP 完整性测试失败：{error}")))
}

fn split_file_to_volumes(
    source_path: &Path,
    first_volume_path: &Path,
    volume_size_mb: u64,
) -> Result<(), ExtractTaskError> {
    if volume_size_mb == 0 {
        return Err(ExtractTaskError::Failed(
            "分卷大小至少为 1 MB。".to_string(),
        ));
    }

    let volume_size = volume_size_mb
        .checked_mul(1024)
        .and_then(|value| value.checked_mul(1024))
        .ok_or_else(|| ExtractTaskError::Failed("分卷大小过大。".to_string()))?;
    let Some(base_name) = first_volume_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".001"))
    else {
        return Err(ExtractTaskError::Failed(
            "无法生成 7Z 分卷文件名。".to_string(),
        ));
    };

    let parent = first_volume_path.parent().unwrap_or_else(|| Path::new(""));
    let mut input = File::open(source_path)
        .map_err(|error| ExtractTaskError::Failed(format!("无法读取 7Z 临时压缩包：{error}")))?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut volume_index = 1_u64;

    loop {
        let output_path = parent.join(format!("{base_name}.{volume_index:03}"));
        let mut output = File::create(&output_path)
            .map_err(|error| ExtractTaskError::Failed(format!("无法创建分卷文件：{error}")))?;
        let mut written = 0_u64;

        while written < volume_size {
            let remaining = (volume_size - written).min(buffer.len() as u64) as usize;
            let read = input.read(&mut buffer[..remaining]).map_err(|error| {
                ExtractTaskError::Failed(format!("读取 7Z 临时压缩包失败：{error}"))
            })?;
            if read == 0 {
                if written == 0 {
                    let _ = fs::remove_file(output_path);
                }
                return Ok(());
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| ExtractTaskError::Failed(format!("写入分卷文件失败：{error}")))?;
            written += read as u64;
        }

        volume_index += 1;
    }
}

fn prepare_compress_plans(options: &CompressOptions) -> Result<Vec<CompressPlan>, String> {
    if options.advanced.batch_queue && options.source_paths.len() > 1 {
        return options
            .source_paths
            .iter()
            .map(|source_path| {
                let mut single = options.clone();
                single.source_paths = vec![source_path.clone()];
                single.archive_name = default_archive_name_for_source(source_path);
                single.advanced.batch_queue = false;
                prepare_compress_plan(&single)
            })
            .collect();
    }
    Ok(vec![prepare_compress_plan(options)?])
}

fn prepare_compress_plan(options: &CompressOptions) -> Result<CompressPlan, String> {
    if options.source_paths.is_empty() {
        return Err("请先选择要压缩的文件或文件夹。".to_string());
    }

    let format = normalize_compress_format(&options.format)?;
    let output_dir = PathBuf::from(options.output_dir.trim());
    if !output_dir.is_dir() {
        return Err("保存位置不存在。".to_string());
    }

    let password = options.password.as_deref().unwrap_or("").trim();
    if !password.is_empty() && !supports_compress_password(&format) {
        return Err("当前仅 ZIP 和 7Z 支持压缩密码。".to_string());
    }
    if options.volume_size_mb.unwrap_or(0) > 0 && !matches!(format.as_str(), "ZIP" | "7Z") {
        return Err("当前仅 ZIP 和 7Z 支持分卷压缩。".to_string());
    }

    let archive_name = sanitize_archive_name(&options.archive_name)?;
    let volume_size_mb = options.volume_size_mb.filter(|value| *value > 0);
    let output_path = if format == "7Z" && volume_size_mb.is_some() {
        unique_split_7z_output_path(&output_dir, &archive_name)
    } else {
        unique_output_path(&output_dir, &archive_name, &format)
    };
    let work_output_path = if format == "7Z" && volume_size_mb.is_some() {
        unique_temp_output_path(&output_dir, &archive_name, &format)
    } else {
        output_path.clone()
    };
    let common_parent = common_parent(&options.source_paths)?;
    let excluded_paths: Vec<PathBuf> = options.excluded_paths.iter().map(PathBuf::from).collect();
    let skip_macos_metadata = options.advanced.skip_macos_metadata;
    let entries = compress_task_entries(
        &options.source_paths,
        &common_parent,
        &excluded_paths,
        options.skip_ds_store,
        skip_macos_metadata,
    )?;
    let total_bytes = entries.iter().map(|entry| entry.size).sum::<u64>().max(1);
    let relative_sources = relative_source_paths(&options.source_paths, &common_parent)?;
    let mut exclude_patterns = excluded_relative_patterns(&excluded_paths, &common_parent)?;
    if options.skip_ds_store {
        append_ds_store_exclude_patterns(&mut exclude_patterns);
    }
    if skip_macos_metadata {
        append_macos_metadata_exclude_patterns(&mut exclude_patterns);
    }

    if format == "ZIP" {
        let args = zip_compress_args(
            &work_output_path,
            &relative_sources,
            &exclude_patterns,
            options.compression_level,
            password,
            volume_size_mb,
        )?;
        return Ok(CompressPlan {
            runner: CompressRunner::Command {
                program: "zip",
                args,
                current_dir: Some(common_parent),
                disable_macos_metadata: skip_macos_metadata,
            },
            output_path,
            work_output_path,
            entries,
            total_bytes,
            format,
            password: (!password.is_empty()).then(|| password.to_string()),
            volume_size_mb,
            test_after_compress: options.advanced.test_after_compress,
        });
    }

    if format == "7Z" {
        let settings = seven_zip_settings(options, password)?;
        let source_paths = options.source_paths.iter().map(PathBuf::from).collect();
        return Ok(CompressPlan {
            runner: CompressRunner::SevenZip {
                common_parent,
                source_paths,
                settings,
            },
            output_path,
            work_output_path,
            entries,
            total_bytes,
            format,
            password: (!password.is_empty()).then(|| password.to_string()),
            volume_size_mb,
            test_after_compress: options.advanced.test_after_compress,
        });
    }

    let args = bsdtar_compress_args(
        &format,
        &work_output_path,
        &common_parent,
        &relative_sources,
        &exclude_patterns,
        options.compression_level,
        &options.advanced,
    )?;
    Ok(CompressPlan {
        runner: CompressRunner::Command {
            program: "bsdtar",
            args,
            current_dir: None,
            disable_macos_metadata: skip_macos_metadata,
        },
        output_path,
        work_output_path,
        entries,
        total_bytes,
        format,
        password: (!password.is_empty()).then(|| password.to_string()),
        volume_size_mb,
        test_after_compress: options.advanced.test_after_compress,
    })
}

fn zip_compress_args(
    output_path: &Path,
    relative_sources: &[OsString],
    exclude_patterns: &[OsString],
    compression_level: Option<u8>,
    password: &str,
    volume_size_mb: Option<u64>,
) -> Result<Vec<OsString>, String> {
    let mut args = vec![OsString::from("-r")];
    if let Some(level) = compression_level {
        if level > 9 {
            return Err("ZIP 压缩级别必须在 0 到 9 之间。".to_string());
        }
        args.push(OsString::from(format!("-{level}")));
    }
    if !password.is_empty() {
        args.push(OsString::from("-P"));
        args.push(OsString::from(password));
    }
    if let Some(volume_size_mb) = volume_size_mb.filter(|value| *value > 0) {
        if volume_size_mb < 1 {
            return Err("分卷大小至少为 1 MB。".to_string());
        }
        args.push(OsString::from("-s"));
        args.push(OsString::from(format!("{volume_size_mb}m")));
    }
    args.push(output_path.as_os_str().to_os_string());
    args.extend(relative_sources.iter().cloned());
    if !exclude_patterns.is_empty() {
        args.push(OsString::from("-x"));
        args.extend(exclude_patterns.iter().cloned());
    }
    Ok(args)
}

fn bsdtar_compress_args(
    format: &str,
    output_path: &Path,
    common_parent: &Path,
    relative_sources: &[OsString],
    exclude_patterns: &[OsString],
    compression_level: Option<u8>,
    _advanced: &CompressAdvancedOptions,
) -> Result<Vec<OsString>, String> {
    let mut args = Vec::new();
    if format == "7Z" {
        args.push(OsString::from("--format"));
        args.push(OsString::from("7zip"));
    }
    if let Some(option) = compression_level_option(format, compression_level)? {
        args.push(OsString::from("--options"));
        args.push(OsString::from(option));
    }
    for pattern in exclude_patterns {
        args.push(OsString::from("--exclude"));
        args.push(pattern.clone());
    }
    args.push(OsString::from(if format == "TAR" || format == "7Z" {
        "-cvf"
    } else {
        "-cvaf"
    }));
    args.push(output_path.as_os_str().to_os_string());
    args.push(OsString::from("-C"));
    args.push(common_parent.as_os_str().to_os_string());
    args.extend(relative_sources.iter().cloned());
    Ok(args)
}

fn compression_level_option(format: &str, level: Option<u8>) -> Result<Option<String>, String> {
    let Some(level) = level else {
        return Ok(None);
    };
    let (module, min, max) = match format {
        "7Z" => ("7zip", 0, 9),
        "TGZ" | "GZ" => ("gzip", 1, 9),
        "TBZ" | "BZ2" => ("bzip2", 1, 9),
        "TXZ" | "XZ" => ("xz", 0, 9),
        "ZSTD" => ("zstd", 1, 22),
        "LZMA" => ("lzma", 0, 9),
        "LZ4" => ("lz4", 1, 2),
        _ => return Ok(None),
    };
    if level < min || level > max {
        return Err(format!(
            "{} 压缩级别必须在 {} 到 {} 之间。",
            format, min, max
        ));
    }
    Ok(Some(format!("{module}:compression-level={level}")))
}

fn supports_compress_password(format: &str) -> bool {
    matches!(format, "ZIP" | "7Z")
}

fn relative_source_paths(paths: &[String], common_parent: &Path) -> Result<Vec<OsString>, String> {
    paths
        .iter()
        .map(|source| {
            let source_path = PathBuf::from(source);
            if !source_path.exists() {
                return Err(format!("找不到文件：{}", source));
            }
            let relative = source_path
                .strip_prefix(common_parent)
                .map_err(|_| "无法计算压缩路径。".to_string())?;
            Ok(relative.as_os_str().to_os_string())
        })
        .collect()
}

fn excluded_relative_patterns(
    excluded_paths: &[PathBuf],
    common_parent: &Path,
) -> Result<Vec<OsString>, String> {
    let mut patterns = Vec::new();
    for excluded_path in excluded_paths {
        if !excluded_path.starts_with(common_parent) {
            continue;
        }
        let relative = excluded_path
            .strip_prefix(common_parent)
            .map_err(|_| "无法计算排除路径。".to_string())?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.is_empty() {
            continue;
        }
        patterns.push(OsString::from(relative.clone()));
        patterns.push(OsString::from(format!("{relative}/*")));
    }
    Ok(patterns)
}

fn append_ds_store_exclude_patterns(patterns: &mut Vec<OsString>) {
    patterns.push(OsString::from(".DS_Store"));
    patterns.push(OsString::from("*/.DS_Store"));
}

fn append_macos_metadata_exclude_patterns(patterns: &mut Vec<OsString>) {
    patterns.push(OsString::from("__MACOSX"));
    patterns.push(OsString::from("__MACOSX/*"));
    patterns.push(OsString::from("*/__MACOSX"));
    patterns.push(OsString::from("*/__MACOSX/*"));
    patterns.push(OsString::from("._*"));
    patterns.push(OsString::from("*/._*"));
}

fn compress_task_entries(
    source_paths: &[String],
    common_parent: &Path,
    excluded_paths: &[PathBuf],
    skip_ds_store: bool,
    skip_macos_metadata: bool,
) -> Result<Vec<ExtractTaskEntry>, String> {
    let mut entries = Vec::new();
    for source in source_paths {
        collect_compress_entry(
            Path::new(source),
            common_parent,
            excluded_paths,
            skip_ds_store,
            skip_macos_metadata,
            &mut entries,
        )?;
    }
    if entries.is_empty() {
        entries.push(ExtractTaskEntry {
            path: "压缩包".to_string(),
            display_path: "压缩包".to_string(),
            size: 1,
            is_dir: false,
            is_unsafe_path: false,
        });
    }
    Ok(entries)
}

fn collect_compress_entry(
    path: &Path,
    common_parent: &Path,
    excluded_paths: &[PathBuf],
    skip_ds_store: bool,
    skip_macos_metadata: bool,
    entries: &mut Vec<ExtractTaskEntry>,
) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("找不到文件：{}", path.to_string_lossy()));
    }
    if is_path_excluded(path, excluded_paths)
        || is_ds_store_path(path, skip_ds_store)
        || is_macos_metadata_path(path, skip_macos_metadata)
    {
        return Ok(());
    }
    let relative = path
        .strip_prefix(common_parent)
        .map_err(|_| "无法计算压缩路径。".to_string())?;
    let display_path = relative.to_string_lossy().replace('\\', "/");
    let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
    let size = if metadata.is_file() {
        metadata.len().max(1)
    } else {
        1
    };
    entries.push(ExtractTaskEntry {
        path: normalize_archive_entry_key(&display_path),
        display_path,
        size,
        is_dir: metadata.is_dir(),
        is_unsafe_path: false,
    });

    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|err| format!("无法读取目录 {}：{err}", path.to_string_lossy()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        children.sort_by_key(|entry| entry.path());
        for child in children {
            collect_compress_entry(
                &child.path(),
                common_parent,
                excluded_paths,
                skip_ds_store,
                skip_macos_metadata,
                entries,
            )?;
        }
    }

    Ok(())
}

fn collect_compress_file_info(
    path: &Path,
    excluded_paths: &[PathBuf],
    skip_ds_store: bool,
    skip_macos_metadata: bool,
    entries: &mut Vec<FileInfo>,
) -> Result<(), String> {
    if !path.exists()
        || is_path_excluded(path, excluded_paths)
        || is_ds_store_path(path, skip_ds_store)
        || is_macos_metadata_path(path, skip_macos_metadata)
    {
        return Ok(());
    }

    entries.push(describe_compress_path(path)?);
    let metadata = fs::metadata(path).map_err(|err| format!("无法读取文件信息：{err}"))?;
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|err| format!("无法读取目录 {}：{err}", path.to_string_lossy()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        children.sort_by_key(|entry| entry.path());
        for child in children {
            collect_compress_file_info(
                &child.path(),
                excluded_paths,
                skip_ds_store,
                skip_macos_metadata,
                entries,
            )?;
        }
    }

    Ok(())
}

fn is_ds_store_path(path: &Path, enabled: bool) -> bool {
    enabled
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(".DS_Store"))
}

fn is_macos_metadata_path(path: &Path, enabled: bool) -> bool {
    if !enabled {
        return false;
    }

    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == "__MACOSX" || name.starts_with("._")
    })
}

fn is_path_excluded(path: &Path, excluded_paths: &[PathBuf]) -> bool {
    excluded_paths
        .iter()
        .any(|excluded_path| path == excluded_path || path.starts_with(excluded_path))
}

fn run_compress_task(task_id: String, task: Arc<ExtractTask>, plans: Vec<CompressPlan>) {
    let mut current_plan_index = 0usize;
    let result = (|| -> Result<(), ExtractTaskError> {
        for (index, plan) in plans.iter().enumerate() {
            current_plan_index = index;
            wait_for_extract_task(&task)?;
            {
                let mut state = task_state(&task)?;
                state.status = "running".to_string();
                state.current_item = file_name(&plan.output_path);
                state.current_bytes = 0;
                state.current_total_bytes = plan.total_bytes.max(1);
                state.message = if plans.len() > 1 {
                    format!(
                        "正在压缩任务 {} / {}：{}",
                        index + 1,
                        plans.len(),
                        file_name(&plan.output_path)
                    )
                } else {
                    "正在压缩。".to_string()
                };
            }

            match &plan.runner {
                CompressRunner::Command {
                    program,
                    args,
                    current_dir,
                    disable_macos_metadata,
                } => run_compress_child_with_progress(
                    &task_id,
                    &task,
                    plan,
                    program,
                    args,
                    current_dir.as_deref(),
                    *disable_macos_metadata,
                )?,
                CompressRunner::SevenZip {
                    common_parent,
                    source_paths,
                    settings,
                } => run_seven_zip_compress_plan(
                    plan,
                    common_parent,
                    source_paths,
                    settings,
                    Some((&task_id, &task)),
                )?,
            }

            finalize_compress_plan(plan, Some((&task_id, &task)))?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            if let Ok(mut state) = task.state.lock() {
                if state.status != "canceled" {
                    state.status = "completed".to_string();
                    state.completed = state.total;
                    state.completed_bytes = state.total_bytes;
                    state.current_bytes = 0;
                    state.current_total_bytes = 0;
                    state.current_item.clear();
                    state.child_pid = None;
                    state.message = format!("压缩完成，已保存到：{}", state.output_path);
                }
            }
        }
        Err(ExtractTaskError::Canceled) => {
            for plan in &plans {
                let _ = remove_partial_compress_plan_outputs(plan);
            }
            if let Ok(mut state) = task.state.lock() {
                state.status = "canceled".to_string();
                state.child_pid = None;
                state.message = "压缩已取消。".to_string();
            }
        }
        Err(ExtractTaskError::Failed(error)) => {
            if let Some(plan) = plans.get(current_plan_index) {
                let _ = remove_partial_compress_plan_outputs(plan);
            }
            if let Ok(mut state) = task.state.lock() {
                state.status = "failed".to_string();
                state.child_pid = None;
                state.error = Some(error.clone());
                state.message = error;
            }
        }
    }
}

fn run_compress_child_with_progress(
    task_id: &str,
    task: &Arc<ExtractTask>,
    plan: &CompressPlan,
    program: &str,
    args: &[OsString],
    current_dir: Option<&Path>,
    disable_macos_metadata: bool,
) -> Result<(), ExtractTaskError> {
    let entries_by_path: HashMap<String, ExtractTaskEntry> = plan
        .entries
        .iter()
        .map(|entry| (normalize_archive_entry_key(&entry.path), entry.clone()))
        .collect();
    let mut seen_entries = BTreeSet::new();
    let candidates = command_candidates(program);

    for (index, candidate) in candidates.iter().enumerate() {
        let mut command = Command::new(candidate);
        command.args(args);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if disable_macos_metadata {
            command.env("COPYFILE_DISABLE", "1");
            command.env("COPY_EXTENDED_ATTRIBUTES_DISABLE", "1");
        }
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }

        match command.spawn() {
            Ok(mut child) => {
                let pid = child.id();
                {
                    let mut state = task_state(task)?;
                    state.child_pid = Some(pid);
                    state.status = "running".to_string();
                    state.message = "正在压缩。".to_string();
                    if state.pause_requested {
                        let _ = send_process_signal(pid, "STOP");
                    }
                }

                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let (sender, receiver) = mpsc::channel::<(bool, String)>();
                let stdout_handle =
                    stdout.map(|stream| spawn_output_reader(stream, false, sender.clone()));
                let stderr_handle = stderr.map(|stream| spawn_output_reader(stream, true, sender));
                let mut recent_stderr: Vec<String> = Vec::new();

                loop {
                    {
                        let mut state = task_state(task)?;
                        if state.cancel_requested {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(ExtractTaskError::Canceled);
                        }
                        if let Some(output_size) = compress_output_size(&plan.work_output_path) {
                            let running_size =
                                output_size.min(state.total_bytes.saturating_sub(1).max(1));
                            state.completed_bytes = state.completed_bytes.max(running_size);
                        }
                    }

                    drain_compress_output(
                        task,
                        &receiver,
                        &entries_by_path,
                        &mut seen_entries,
                        &mut recent_stderr,
                    )?;

                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if let Some(handle) = stdout_handle {
                                let _ = handle.join();
                            }
                            if let Some(handle) = stderr_handle {
                                let _ = handle.join();
                            }
                            drain_compress_output(
                                task,
                                &receiver,
                                &entries_by_path,
                                &mut seen_entries,
                                &mut recent_stderr,
                            )?;
                            if status.success() {
                                return Ok(());
                            }
                            let message = recent_stderr
                                .into_iter()
                                .filter(|line| !line.trim().is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                            return Err(ExtractTaskError::Failed(if message.is_empty() {
                                format!("{program} 执行失败。")
                            } else {
                                message
                            }));
                        }
                        Ok(None) => {
                            thread::sleep(Duration::from_millis(120));
                        }
                        Err(error) => {
                            return Err(ExtractTaskError::Failed(format!(
                                "压缩任务 {task_id} 状态读取失败：{error}"
                            )));
                        }
                    }
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && index + 1 < candidates.len() => {
            }
            Err(_) => {
                return Err(ExtractTaskError::Failed(format!(
                    "找不到命令：{}。请确认系统已安装对应压缩工具。",
                    program
                )));
            }
        }
    }

    Err(ExtractTaskError::Failed(format!(
        "找不到命令：{}。请确认系统已安装对应压缩工具。",
        program
    )))
}

fn run_seven_zip_compress_plan(
    plan: &CompressPlan,
    common_parent: &Path,
    _source_paths: &[PathBuf],
    settings: &SevenZipCompressSettings,
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    use sevenz_rust2::ArchiveWriter as SevenZipWriter;

    let mut writer = SevenZipWriter::create(&plan.work_output_path)
        .map_err(|error| ExtractTaskError::Failed(format!("无法创建 7Z 压缩包：{error}")))?;
    writer.set_encrypt_header(settings.password.is_some());
    writer.set_content_methods(seven_zip_content_methods(settings));

    if settings.solid {
        run_solid_seven_zip_compress(plan, common_parent, &mut writer, progress)?;
    } else {
        run_non_solid_seven_zip_compress(plan, common_parent, &mut writer, progress)?;
    }

    writer
        .finish()
        .map_err(|error| ExtractTaskError::Failed(format!("无法完成 7Z 压缩：{error}")))?;

    Ok(())
}

fn seven_zip_content_methods(
    settings: &SevenZipCompressSettings,
) -> Vec<sevenz_rust2::EncoderConfiguration> {
    use sevenz_rust2::{
        encoder_options::{AesEncoderOptions, EncoderOptions, Lzma2Options, LzmaOptions},
        EncoderConfiguration, EncoderMethod, Password as SevenZipPassword,
    };

    let mut methods = Vec::new();
    if let Some(password) = &settings.password {
        methods.push(AesEncoderOptions::new(SevenZipPassword::new(password)).into());
    }

    match settings.method {
        SevenZipMethod::Lzma2 => {
            let mut options = if settings.threads > 1 {
                let chunk_size = settings
                    .dictionary_size_mb
                    .map(|mb| u64::from(mb) * 1024 * 1024)
                    .unwrap_or(16 * 1024 * 1024);
                Lzma2Options::from_level_mt(
                    settings.compression_level as u32,
                    settings.threads,
                    chunk_size,
                )
            } else {
                Lzma2Options::from_level(settings.compression_level as u32)
            };
            if let Some(dictionary_size_mb) = settings.dictionary_size_mb {
                options.set_dictionary_size(dictionary_size_mb.saturating_mul(1024 * 1024));
            }
            methods.push(options.into());
        }
        SevenZipMethod::Lzma => {
            methods.push(EncoderConfiguration::new(EncoderMethod::LZMA).with_options(
                EncoderOptions::Lzma(LzmaOptions::from_level(settings.compression_level as u32)),
            ));
        }
    }

    methods
}

fn run_non_solid_seven_zip_compress(
    plan: &CompressPlan,
    common_parent: &Path,
    writer: &mut sevenz_rust2::ArchiveWriter<File>,
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    use sevenz_rust2::ArchiveEntry as SevenZipEntry;

    for entry in &plan.entries {
        begin_compress_entry_progress(entry, progress)?;

        let source_path = common_parent.join(Path::new(&entry.path));
        if source_path.is_dir() {
            let archive_entry = SevenZipEntry::from_path(&source_path, entry.path.clone());
            writer
                .push_archive_entry::<&[u8]>(archive_entry, None)
                .map_err(|error| {
                    ExtractTaskError::Failed(format!("无法写入 7Z 文件夹：{error}"))
                })?;
        } else if source_path.is_file() {
            let archive_entry = SevenZipEntry::from_path(&source_path, entry.path.clone());
            let file = File::open(&source_path)
                .map_err(|error| ExtractTaskError::Failed(format!("无法读取文件：{error}")))?;
            writer
                .push_archive_entry(archive_entry, Some(file))
                .map_err(|error| ExtractTaskError::Failed(format!("无法写入 7Z 文件：{error}")))?;
        }

        complete_compress_entry_progress(entry, progress)?;
    }

    Ok(())
}

fn run_solid_seven_zip_compress(
    plan: &CompressPlan,
    common_parent: &Path,
    writer: &mut sevenz_rust2::ArchiveWriter<File>,
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    use sevenz_rust2::{ArchiveEntry as SevenZipEntry, SourceReader as SevenZipSourceReader};

    let mut solid_entries = Vec::new();
    let mut solid_readers = Vec::new();
    let mut solid_task_entries = Vec::new();

    for entry in &plan.entries {
        let source_path = common_parent.join(Path::new(&entry.path));
        if source_path.is_dir() {
            begin_compress_entry_progress(entry, progress)?;
            let archive_entry = SevenZipEntry::from_path(&source_path, entry.path.clone());
            writer
                .push_archive_entry::<&[u8]>(archive_entry, None)
                .map_err(|error| {
                    ExtractTaskError::Failed(format!("无法写入 7Z 文件夹：{error}"))
                })?;
            complete_compress_entry_progress(entry, progress)?;
        } else if source_path.is_file() {
            solid_entries.push(SevenZipEntry::from_path(&source_path, entry.path.clone()));
            let file = File::open(&source_path)
                .map_err(|error| ExtractTaskError::Failed(format!("无法读取文件：{error}")))?;
            solid_readers.push(SevenZipSourceReader::new(file));
            solid_task_entries.push(entry.clone());
        }
    }

    if solid_entries.is_empty() {
        return Ok(());
    }

    if let Some((_, task)) = progress {
        wait_for_extract_task(task)?;
        let mut state = task_state(task)?;
        state.status = "running".to_string();
        state.current_item = "固实压缩数据块".to_string();
        state.current_bytes = 0;
        state.current_total_bytes = solid_task_entries
            .iter()
            .map(|entry| entry.size.max(1))
            .sum::<u64>()
            .max(1);
        state.message = format!("正在固实压缩 {} 个文件", solid_task_entries.len());
    }

    writer
        .push_archive_entries(solid_entries, solid_readers)
        .map_err(|error| ExtractTaskError::Failed(format!("无法写入 7Z 固实数据块：{error}")))?;

    complete_compress_entries_progress(&solid_task_entries, progress)?;
    Ok(())
}

fn begin_compress_entry_progress(
    entry: &ExtractTaskEntry,
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    if let Some((_, task)) = progress {
        wait_for_extract_task(task)?;
        let mut state = task_state(task)?;
        state.status = "running".to_string();
        state.current_item = entry.display_path.clone();
        state.current_bytes = 0;
        state.current_total_bytes = entry.size.max(1);
        state.message = format!(
            "正在压缩 {} / {}",
            state.completed.saturating_add(1),
            state.total
        );
    }
    Ok(())
}

fn complete_compress_entry_progress(
    entry: &ExtractTaskEntry,
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    complete_compress_entries_progress(std::slice::from_ref(entry), progress)
}

fn complete_compress_entries_progress(
    entries: &[ExtractTaskEntry],
    progress: Option<(&str, &Arc<ExtractTask>)>,
) -> Result<(), ExtractTaskError> {
    if let Some((_, task)) = progress {
        let mut state = task_state(task)?;
        state.completed = state
            .completed
            .saturating_add(entries.len())
            .min(state.total);
        state.completed_bytes = state
            .completed_bytes
            .saturating_add(entries.iter().map(|entry| entry.size.max(1)).sum::<u64>())
            .min(state.total_bytes);
        state.current_bytes = 0;
    }
    Ok(())
}

fn spawn_output_reader<R: Read + Send + 'static>(
    stream: R,
    is_stderr: bool,
    sender: mpsc::Sender<(bool, String)>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if sender.send((is_stderr, line)).is_err() {
                break;
            }
        }
    })
}

fn drain_compress_output(
    task: &Arc<ExtractTask>,
    receiver: &mpsc::Receiver<(bool, String)>,
    entries_by_path: &HashMap<String, ExtractTaskEntry>,
    seen_entries: &mut BTreeSet<String>,
    recent_stderr: &mut Vec<String>,
) -> Result<(), ExtractTaskError> {
    while let Ok((is_stderr, line)) = receiver.try_recv() {
        if is_stderr {
            recent_stderr.push(line.clone());
            if recent_stderr.len() > 12 {
                recent_stderr.remove(0);
            }
        }
        update_compress_progress_from_line(task, &line, entries_by_path, seen_entries)?;
    }
    Ok(())
}

fn update_compress_progress_from_line(
    task: &Arc<ExtractTask>,
    line: &str,
    entries_by_path: &HashMap<String, ExtractTaskEntry>,
    seen_entries: &mut BTreeSet<String>,
) -> Result<(), ExtractTaskError> {
    let Some(path) = parse_compress_progress_line(line) else {
        return Ok(());
    };
    let key = normalize_archive_entry_key(&path);
    let matched = entries_by_path.get(&key);
    let mut state = task_state(task)?;
    state.current_item = matched
        .map(|entry| entry.display_path.clone())
        .unwrap_or(path);
    if let Some(entry) = matched {
        state.current_total_bytes = entry.size.max(1);
        state.current_bytes = entry.size.max(1);
        if seen_entries.insert(key) {
            state.completed = state.completed.saturating_add(1).min(state.total);
            state.completed_bytes = state
                .completed_bytes
                .saturating_add(entry.size.max(1))
                .min(state.total_bytes);
        }
        state.message = format!(
            "正在压缩 {} / {}",
            state.completed.max(1).min(state.total),
            state.total
        );
    }
    Ok(())
}

fn parse_compress_progress_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("a ") {
        return Some(rest.trim().to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("+ ") {
        return Some(rest.trim().to_string());
    }
    for prefix in ["adding:", "updating:", "freshening:"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let path = rest.trim();
            let path = path.split(" (").next().unwrap_or(path).trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    for prefix in ["Compressing  ", "Compressing ", "Adding  ", "Adding "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let path = rest.trim();
            if !path.is_empty() {
                return Some(path.to_string());
            }
        }
    }
    None
}

fn normalize_archive_entry_key(path: &str) -> String {
    path.trim()
        .trim_start_matches("./")
        .trim_end_matches('/')
        .replace('\\', "/")
}

fn compress_output_size(path: &Path) -> Option<u64> {
    let mut total = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path == path {
                    continue;
                }
                let matches_stem = entry_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|value| value == stem)
                    .unwrap_or(false);
                let matches_split_extension = entry_path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|extension| {
                        let lower = extension.to_ascii_lowercase();
                        lower.len() == 3
                            && lower.starts_with('z')
                            && lower[1..].chars().all(|char| char.is_ascii_digit())
                    })
                    .unwrap_or(false);
                if matches_stem && matches_split_extension {
                    total = total.saturating_add(
                        fs::metadata(entry_path)
                            .map(|metadata| metadata.len())
                            .unwrap_or(0),
                    );
                }
            }
        }
    }
    if total == 0 {
        None
    } else {
        Some(total)
    }
}

fn remove_partial_compress_outputs(path: &Path) -> Result<(), String> {
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
    {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        let stem = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                let matches_stem = entry_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|value| value == stem)
                    .unwrap_or(false);
                let matches_split_extension = entry_path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|extension| {
                        let lower = extension.to_ascii_lowercase();
                        lower.len() == 3
                            && lower.starts_with('z')
                            && lower[1..].chars().all(|char| char.is_ascii_digit())
                    })
                    .unwrap_or(false);
                if matches_stem && matches_split_extension {
                    let _ = fs::remove_file(entry_path);
                }
            }
        }
    }
    if let Some((base_name, _)) = path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(numeric_split_volume_number)
    {
        let parent = path.parent().unwrap_or_else(|| Path::new(""));
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                let Some(name) = entry_path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                let Some((candidate_base, _)) = numeric_split_volume_number(name) else {
                    continue;
                };
                if candidate_base == base_name {
                    let _ = fs::remove_file(entry_path);
                }
            }
        }
    }
    Ok(())
}

fn remove_partial_compress_plan_outputs(plan: &CompressPlan) -> Result<(), String> {
    remove_partial_compress_outputs(&plan.output_path)?;
    if plan.work_output_path != plan.output_path {
        remove_partial_compress_outputs(&plan.work_output_path)?;
    }
    Ok(())
}

fn normalize_archive_open_path(path: &Path) -> Result<PathBuf, String> {
    let name = file_name(path);
    if zip_z_volume_number(&name.to_ascii_lowercase()).is_some() {
        ensure_file(path)?;
        let main_path = zip_split_main_path(path);
        if main_path.is_file() {
            return Ok(main_path);
        }
        let main_name = main_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("同名 .zip 主文件");
        return Err(format!(
            "这是 ZIP 分卷片段，但找不到同名主文件 {main_name}。请把 .z01/.z02 和 .zip 放在同一文件夹后再打开。"
        ));
    }

    Ok(path.to_path_buf())
}

fn prepare_archive_for_read(archive_path: &Path) -> Result<PreparedArchive, String> {
    if archive_format(archive_path) != "ZIP" {
        return Ok(PreparedArchive::borrowed(archive_path));
    }

    let volumes = archive_split_volumes(archive_path);
    if volumes.len() <= 1 {
        return Ok(PreparedArchive::borrowed(archive_path));
    }

    let temp_dir = zip_split_merge_temp_dir(archive_path);
    fs::create_dir_all(&temp_dir).map_err(|err| format!("无法创建 ZIP 分卷临时目录：{err}"))?;
    let merged_path = temp_dir.join(file_name(archive_path));
    merge_zip_split_archive(archive_path, &merged_path)?;
    Ok(PreparedArchive {
        path: merged_path,
        temp_dir: Some(temp_dir),
    })
}

fn merge_zip_split_archive(archive_path: &Path, output_path: &Path) -> Result<(), String> {
    let volumes = archive_split_volumes(archive_path);
    if volumes.len() <= 1 {
        fs::copy(archive_path, output_path).map_err(|err| format!("无法准备 ZIP 压缩包：{err}"))?;
        return Ok(());
    }

    let mut output =
        File::create(output_path).map_err(|err| format!("无法创建 ZIP 分卷临时文件：{err}"))?;
    for volume in volumes {
        let mut input = File::open(&volume).map_err(|err| {
            format!(
                "无法读取 ZIP 分卷 {}：{err}",
                volume
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("未知分卷")
            )
        })?;
        std::io::copy(&mut input, &mut output)
            .map_err(|err| format!("无法合并 ZIP 分卷：{err}"))?;
    }
    output
        .flush()
        .map_err(|err| format!("无法写入 ZIP 分卷临时文件：{err}"))?;
    Ok(())
}

fn test_archive_by_extracting_to_temp(
    archive_path: &Path,
    password: Option<&str>,
) -> Result<(), String> {
    let temp_dir = archive_integrity_temp_dir(archive_path);
    fs::create_dir_all(&temp_dir).map_err(|err| format!("无法创建完整性测试临时目录：{err}"))?;
    let result = (|| {
        let mut args = Vec::new();
        append_archive_extract_password_args(&mut args, password);
        args.push(OsString::from("-xf"));
        args.push(archive_path.as_os_str().to_os_string());
        args.push(OsString::from("-C"));
        args.push(temp_dir.as_os_str().to_os_string());
        run_command("bsdtar", args.iter().map(|arg| arg.as_os_str()), None).map(|_| ())
    })();
    let _ = fs::remove_dir_all(&temp_dir);
    result
}

fn archive_integrity_temp_dir(archive_path: &Path) -> PathBuf {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let index = EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join("cardiganzip-integrity")
        .join(format!(
            "{}-{seed}-{index}",
            sanitize_path_segment(&file_name(archive_path))
        ))
}

fn zip_split_merge_temp_dir(archive_path: &Path) -> PathBuf {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let index = EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join("cardiganzip-zip-split")
        .join(format!(
            "{}-{seed}-{index}",
            sanitize_path_segment(&file_name(archive_path))
        ))
}

fn test_archive_integrity_path(archive_path: &Path, password: Option<&str>) -> Result<(), String> {
    if is_single_stream_format(archive_path) {
        let (program, args) = single_stream_decompress_command(archive_path)?;
        run_command_bytes(program, args.iter().map(|arg| arg.as_os_str()), None)?;
        return Ok(());
    }

    match archive_format(archive_path).as_str() {
        "ZIP" if archive_split_volumes(archive_path).len() > 1 => {
            let prepared_archive = prepare_archive_for_read(archive_path)?;
            test_archive_by_extracting_to_temp(prepared_archive.path(), password)
        }
        "ZIP" => {
            let mut args = vec![OsString::from("-t")];
            args.push(OsString::from("-P"));
            args.push(OsString::from(
                password
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or(ARCHIVE_PASSWORD_PROBE),
            ));
            args.push(archive_path.as_os_str().to_os_string());
            run_command("unzip", args.iter().map(|arg| arg.as_os_str()), None).map(|_| ())
        }
        "7Z" => {
            use sevenz_rust2::{ArchiveReader, Password as SevenZipPassword};

            let password = password
                .map(SevenZipPassword::new)
                .unwrap_or_else(SevenZipPassword::empty);
            ArchiveReader::open(archive_path, password)
                .map(|_| ())
                .map_err(|error| format!("7Z 完整性测试失败：{error}"))
        }
        _ => {
            let mut args = Vec::new();
            append_archive_extract_password_args(&mut args, password);
            args.push(OsString::from("-tf"));
            args.push(archive_path.as_os_str().to_os_string());
            run_command("bsdtar", args.iter().map(|arg| arg.as_os_str()), None).map(|_| ())
        }
    }
}

fn ensure_archive_is_editable(archive_path: &Path) -> Result<(), String> {
    if archive_split_volumes(archive_path).len() > 1 || split_archive_format(archive_path).is_some()
    {
        return Err("暂不支持直接编辑分卷压缩包，请先合并或解压后重新压缩。".to_string());
    }
    let format = archive_format(archive_path);
    normalize_compress_format(&format)
        .map(|_| ())
        .map_err(|_| "当前格式暂不支持编辑后重建。".to_string())
}

fn normalize_edit_archive_format(
    archive_path: &Path,
    output_path: Option<&str>,
) -> Result<String, String> {
    if let Some(output_path) = output_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(Path::new)
    {
        let output_format = archive_format(output_path);
        if normalize_compress_format(&output_format).is_ok() {
            return normalize_compress_format(&output_format);
        }
    }
    normalize_compress_format(&archive_format(archive_path))
}

fn archive_edit_temp_dir(archive_path: &Path) -> PathBuf {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let index = EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join("cardiganzip-edit").join(format!(
        "{}-{seed}-{index}",
        sanitize_path_segment(&file_name(archive_path))
    ))
}

fn safe_archive_relative_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("路径不能为空。".to_string());
    }
    if archive_entry_has_unsafe_path(trimmed) {
        return Err(format!("路径不安全：{trimmed}"));
    }

    let path = strip_archive_entry_prefix(trimmed);
    if path.as_os_str().is_empty() {
        return Err("路径不能为空。".to_string());
    }
    Ok(path)
}

fn apply_archive_deletes(content_dir: &Path, entries: &[String]) -> Result<(), String> {
    for entry in entries {
        let relative = safe_archive_relative_path(entry)?;
        let target = content_dir.join(relative);
        if target.is_dir() {
            fs::remove_dir_all(&target).map_err(|err| format!("无法删除文件夹：{err}"))?;
        } else if target.exists() {
            fs::remove_file(&target).map_err(|err| format!("无法删除文件：{err}"))?;
        }
    }
    Ok(())
}

fn apply_archive_renames(content_dir: &Path, entries: &[ArchiveRenameEntry]) -> Result<(), String> {
    for entry in entries {
        let from = content_dir.join(safe_archive_relative_path(&entry.from)?);
        let to_relative = safe_archive_relative_path(&entry.to)?;
        let to = content_dir.join(to_relative);
        if !from.exists() {
            return Err(format!("找不到要重命名的条目：{}", entry.from));
        }
        if to.exists() {
            return Err(format!("目标路径已存在：{}", entry.to));
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("无法创建目标目录：{err}"))?;
        }
        fs::rename(&from, &to).map_err(|err| format!("无法重命名条目：{err}"))?;
    }
    Ok(())
}

fn apply_archive_replacements(
    content_dir: &Path,
    entries: &[ArchiveReplaceEntry],
) -> Result<(), String> {
    for entry in entries {
        let target = content_dir.join(safe_archive_relative_path(&entry.entry_path)?);
        let source = PathBuf::from(entry.source_path.trim());
        if !source.exists() {
            return Err(format!("找不到替换来源：{}", entry.source_path));
        }
        if target.is_dir() {
            fs::remove_dir_all(&target).map_err(|err| format!("无法移除旧文件夹：{err}"))?;
        } else if target.exists() {
            fs::remove_file(&target).map_err(|err| format!("无法移除旧文件：{err}"))?;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("无法创建目标目录：{err}"))?;
        }
        copy_path_recursive(&source, &target)?;
    }
    Ok(())
}

fn apply_archive_additions(content_dir: &Path, paths: &[String]) -> Result<(), String> {
    for path in paths {
        add_path_to_archive_dir(content_dir, path, "")?;
    }
    Ok(())
}

fn apply_archive_add_entries(
    content_dir: &Path,
    entries: &[ArchiveAddEntry],
) -> Result<(), String> {
    for entry in entries {
        add_path_to_archive_dir(content_dir, &entry.source_path, &entry.target_dir)?;
    }
    Ok(())
}

fn apply_archive_create_dirs(content_dir: &Path, dirs: &[String]) -> Result<(), String> {
    for dir in dirs {
        let relative = safe_archive_relative_path(dir)?;
        let target = content_dir.join(relative);
        if target.exists() {
            return Err(format!("目标文件夹已存在：{dir}"));
        }
        fs::create_dir_all(&target).map_err(|err| format!("无法创建文件夹：{err}"))?;
    }
    Ok(())
}

fn add_path_to_archive_dir(
    content_dir: &Path,
    source_path: &str,
    target_dir: &str,
) -> Result<(), String> {
    let source = PathBuf::from(source_path.trim());
    if !source.exists() {
        return Err(format!("找不到要添加的项目：{source_path}"));
    }

    let target_parent = archive_add_target_parent(content_dir, target_dir)?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_path_segment)
        .unwrap_or_else(|| "未命名".to_string());
    let target = unique_archive_edit_child_path(&target_parent, &name);
    copy_path_recursive(&source, &target)
}

fn archive_add_target_parent(content_dir: &Path, target_dir: &str) -> Result<PathBuf, String> {
    let trimmed = target_dir.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Ok(content_dir.to_path_buf());
    }

    let relative = safe_archive_relative_path(trimmed)?;
    let target = content_dir.join(relative);
    if target.exists() && !target.is_dir() {
        return Err(format!("添加目标不是文件夹：{target_dir}"));
    }
    fs::create_dir_all(&target).map_err(|err| format!("无法创建添加目标文件夹：{err}"))?;
    Ok(target)
}

fn unique_archive_edit_child_path(parent: &Path, name: &str) -> PathBuf {
    let candidate = parent.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty());
    let mut index = 2;
    loop {
        let file_name = if let Some(extension) = extension {
            format!("{stem} {index}.{extension}")
        } else {
            format!("{stem} {index}")
        };
        let candidate = parent.join(file_name);
        if !candidate.exists() {
            return candidate;
        }
        index += 1;
    }
}

fn copy_path_recursive(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|err| format!("无法读取来源：{err}"))?;
    if metadata.is_dir() {
        fs::create_dir_all(target).map_err(|err| format!("无法创建文件夹：{err}"))?;
        let mut children = fs::read_dir(source)
            .map_err(|err| format!("无法读取文件夹：{err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        children.sort_by_key(|entry| entry.path());
        for child in children {
            copy_path_recursive(&child.path(), &target.join(child.file_name()))?;
        }
    } else if metadata.is_file() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("无法创建目标目录：{err}"))?;
        }
        fs::copy(source, target).map_err(|err| format!("无法复制文件：{err}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_copyable_temp_tree_permissions(path: &Path) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|err| format!("无法读取临时解压项目：{err}"))?;

    if metadata.is_dir() {
        ensure_temp_path_owner_permissions(path, &metadata, 0o700)?;
        let mut children = fs::read_dir(path)
            .map_err(|err| format!("无法读取临时解压文件夹：{err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        children.sort_by_key(|entry| entry.path());
        for child in children {
            ensure_copyable_temp_tree_permissions(&child.path())?;
        }
    } else if metadata.is_file() {
        ensure_temp_path_owner_permissions(path, &metadata, 0o400)?;
    }

    Ok(())
}

#[cfg(unix)]
fn ensure_temp_path_owner_permissions(
    path: &Path,
    metadata: &fs::Metadata,
    required_owner_bits: u32,
) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode();
    if mode & required_owner_bits == required_owner_bits {
        return Ok(());
    }

    let mut permissions = metadata.permissions();
    permissions.set_mode(mode | required_owner_bits);
    fs::set_permissions(path, permissions).map_err(|err| format!("无法修正临时解压权限：{err}"))
}

#[cfg(not(unix))]
fn ensure_copyable_temp_tree_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn archive_edit_source_paths(content_dir: &Path) -> Result<Vec<String>, String> {
    let mut children = fs::read_dir(content_dir)
        .map_err(|err| format!("无法读取编辑内容：{err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    children.sort_by_key(|entry| entry.path());
    Ok(children
        .into_iter()
        .map(|entry| entry.path().to_string_lossy().to_string())
        .collect())
}

fn install_rebuilt_archive(rebuilt_path: &Path, output_path: &Path) -> Result<(), String> {
    let parent = output_path
        .parent()
        .ok_or_else(|| "无法确定保存位置。".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("无法创建保存位置：{err}"))?;

    if output_path.exists() {
        let backup =
            unique_temp_output_path(parent, ".packo-edit-backup", &archive_format(output_path));
        fs::rename(output_path, &backup).map_err(|err| format!("无法备份原压缩包：{err}"))?;
        match fs::rename(rebuilt_path, output_path) {
            Ok(()) => {
                let _ = fs::remove_file(backup);
                Ok(())
            }
            Err(error) => {
                let _ = fs::rename(&backup, output_path);
                Err(format!("无法保存编辑后的压缩包：{error}"))
            }
        }
    } else {
        fs::rename(rebuilt_path, output_path)
            .map_err(|err| format!("无法保存编辑后的压缩包：{err}"))
    }
}

#[tauri::command]
fn extract_archive(
    path: String,
    output_dir: String,
    password: Option<String>,
) -> Result<OperationResult, String> {
    extract_archive_items(path, output_dir, Vec::new(), password)
}

#[tauri::command]
fn extract_archive_entries(
    path: String,
    output_dir: String,
    entries: Vec<String>,
    password: Option<String>,
) -> Result<OperationResult, String> {
    extract_archive_items(path, output_dir, entries, password)
}

#[tauri::command]
fn start_extract_task(
    tasks: tauri::State<'_, ExtractTasks>,
    path: String,
    output_dir: String,
    entries: Vec<String>,
    password: Option<String>,
    conflict_strategy: Option<String>,
) -> Result<ExtractTaskProgress, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
    let password = normalize_archive_password(password);
    let conflict_strategy = normalize_extract_conflict_strategy(conflict_strategy)?;
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;

    let output_path = PathBuf::from(output_dir.trim());
    if output_path.as_os_str().is_empty() {
        return Err("请选择解压位置。".to_string());
    }
    fs::create_dir_all(&output_path).map_err(|err| format!("无法创建解压目录：{err}"))?;

    let task_entries = extract_task_entries(&archive_path, entries, password.as_deref())?;
    let total = task_entries.len().max(1);
    let total_bytes = task_entries
        .iter()
        .map(|entry| entry.size)
        .sum::<u64>()
        .max(1);
    let task_id = format!(
        "extract-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let task = Arc::new(ExtractTask::new(
        total,
        total_bytes,
        output_path.to_string_lossy().to_string(),
    ));

    tasks
        .tasks
        .lock()
        .map_err(|_| "无法创建解压任务。".to_string())?
        .insert(task_id.clone(), Arc::clone(&task));

    let thread_task_id = task_id.clone();
    thread::spawn(move || {
        run_extract_task(
            thread_task_id,
            task,
            archive_path,
            output_path,
            task_entries,
            password,
            conflict_strategy,
        );
    });

    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn start_batch_extract_task(
    tasks: tauri::State<'_, ExtractTasks>,
    paths: Vec<String>,
    output_dir: String,
    password: Option<String>,
    conflict_strategy: Option<String>,
) -> Result<ExtractTaskProgress, String> {
    let password = normalize_archive_password(password);
    let conflict_strategy = normalize_extract_conflict_strategy(conflict_strategy)?;
    let output_path = PathBuf::from(output_dir.trim());
    if output_path.as_os_str().is_empty() {
        return Err("请选择解压位置。".to_string());
    }
    fs::create_dir_all(&output_path).map_err(|err| format!("无法创建解压目录：{err}"))?;

    let (plans, total, total_bytes) =
        prepare_batch_extract_plans(paths, &output_path, password.as_deref())?;
    let task_id = format!(
        "batch-extract-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let task = Arc::new(ExtractTask::new(
        total,
        total_bytes,
        output_path.to_string_lossy().to_string(),
    ));

    tasks
        .tasks
        .lock()
        .map_err(|_| "无法创建解压任务。".to_string())?
        .insert(task_id.clone(), Arc::clone(&task));

    let thread_task_id = task_id.clone();
    thread::spawn(move || {
        run_batch_extract_task(thread_task_id, task, plans, password, conflict_strategy);
    });

    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn get_extract_task(
    tasks: tauri::State<'_, ExtractTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn pause_extract_task(
    tasks: tauri::State<'_, ExtractTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    let task = find_extract_task(&tasks, &task_id)?;
    let child_pid = {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法暂停解压任务。".to_string())?;
        if state.status == "completed" || state.status == "failed" || state.status == "canceled" {
            return Ok(extract_task_progress(&task_id, &state));
        }
        state.pause_requested = true;
        state.status = "paused".to_string();
        state.message = "解压已暂停。".to_string();
        state.child_pid
    };

    if let Some(pid) = child_pid {
        let _ = send_process_signal(pid, "STOP");
    }

    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn resume_extract_task(
    tasks: tauri::State<'_, ExtractTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    let task = find_extract_task(&tasks, &task_id)?;
    let child_pid = {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法继续解压任务。".to_string())?;
        if state.status == "completed" || state.status == "failed" || state.status == "canceled" {
            return Ok(extract_task_progress(&task_id, &state));
        }
        state.pause_requested = false;
        state.status = "running".to_string();
        state.message = "正在继续解压。".to_string();
        state.child_pid
    };

    if let Some(pid) = child_pid {
        let _ = send_process_signal(pid, "CONT");
    }

    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn cancel_extract_task(
    tasks: tauri::State<'_, ExtractTasks>,
    task_id: String,
) -> Result<ExtractTaskProgress, String> {
    let task = find_extract_task(&tasks, &task_id)?;
    let child_pid = {
        let mut state = task
            .state
            .lock()
            .map_err(|_| "无法取消解压任务。".to_string())?;
        if state.status == "completed" || state.status == "failed" || state.status == "canceled" {
            return Ok(extract_task_progress(&task_id, &state));
        }
        state.cancel_requested = true;
        state.pause_requested = false;
        state.status = "canceling".to_string();
        state.message = "正在取消解压。".to_string();
        state.child_pid
    };

    if let Some(pid) = child_pid {
        let _ = send_process_signal(pid, "KILL");
    }

    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn preview_archive_entry(
    path: String,
    entry_path: String,
    password: Option<String>,
) -> Result<PreviewResult, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
    let password = normalize_archive_password(password);
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;

    let preview_dir = preview_output_dir(&archive_path)?;
    fs::create_dir_all(&preview_dir).map_err(|err| format!("无法创建预览目录：{err}"))?;

    let extracted_paths = extract_entries_to_dir(
        &archive_path,
        &preview_dir,
        &[entry_path.clone()],
        password.as_deref(),
    )?;
    let output_path = extracted_paths
        .first()
        .cloned()
        .unwrap_or_else(|| preview_dir.join(strip_archive_entry_prefix(&entry_path)));

    Ok(PreviewResult {
        output_path: output_path.to_string_lossy().to_string(),
        message: "已解压到临时预览位置。".to_string(),
    })
}

#[tauri::command]
fn suggest_open_with_apps(path: String) -> Result<Vec<OpenWithApplication>, String> {
    let path = PathBuf::from(path);
    ensure_file(&path)?;
    Ok(macos_open_with::suggest_open_with_apps(&path))
}

#[tauri::command]
fn open_file_with_application(
    path: String,
    application_path: Option<String>,
) -> Result<(), String> {
    let path = PathBuf::from(path);
    ensure_file(&path)?;
    let application_path = application_path.map(PathBuf::from);
    macos_open_with::open_file_with_application(&path, application_path.as_deref())
}

#[tauri::command]
fn start_preview_task(
    tasks: tauri::State<'_, ExtractTasks>,
    path: String,
    entry_path: String,
    password: Option<String>,
) -> Result<ExtractTaskProgress, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
    let password = normalize_archive_password(password);
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;

    if entry_path.trim().is_empty() || entry_path.ends_with('/') {
        return Err("请选择要预览的文件。".to_string());
    }

    let preview_dir = preview_output_dir(&archive_path)?;
    fs::create_dir_all(&preview_dir).map_err(|err| format!("无法创建预览目录：{err}"))?;

    let task_entries =
        extract_task_entries(&archive_path, vec![entry_path.clone()], password.as_deref())?;
    let total = task_entries.len().max(1);
    let total_bytes = task_entries
        .iter()
        .map(|entry| entry.size)
        .sum::<u64>()
        .max(1);
    let output_path = preview_dir.join(strip_archive_entry_prefix(&entry_path));
    let task_id = format!(
        "preview-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
        EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let task = Arc::new(ExtractTask::new(
        total,
        total_bytes,
        output_path.to_string_lossy().to_string(),
    ));

    tasks
        .tasks
        .lock()
        .map_err(|_| "无法创建预览任务。".to_string())?
        .insert(task_id.clone(), Arc::clone(&task));

    let thread_task_id = task_id.clone();
    thread::spawn(move || {
        run_extract_task(
            thread_task_id,
            task,
            archive_path,
            preview_dir,
            task_entries,
            password,
            ExtractConflictStrategy::Rename,
        );
    });

    get_extract_task_progress(&tasks, &task_id)
}

#[tauri::command]
fn start_archive_entry_promise_drag<R: tauri::Runtime>(
    window: tauri::Window<R>,
    path: String,
    entry_path: String,
    suggested_name: String,
    password: Option<String>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
        let password = normalize_archive_password(password);
        ensure_file(&archive_path)?;
        ensure_supported_extract_format(&archive_path)?;

        if entry_path.trim().is_empty() || entry_path.ends_with('/') {
            return Err("请选择要拖出的文件。".to_string());
        }
        if archive_entry_has_unsafe_path(&entry_path) {
            return Err("该条目路径不安全，已阻止拖出。".to_string());
        }

        let promised_name = promised_archive_entry_name(&entry_path, &suggested_name);
        macos_file_promise_drag::start_archive_entry_promise_drag(
            &window,
            archive_path,
            entry_path,
            promised_name,
            false,
            password,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, path, entry_path, suggested_name, password);
        Err("当前仅支持在 macOS 上拖出压缩包内文件。".to_string())
    }
}

#[tauri::command]
fn start_archive_entries_promise_drag<R: tauri::Runtime>(
    window: tauri::Window<R>,
    path: String,
    items: Vec<ArchivePromiseDragItem>,
    password: Option<String>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
        let password = normalize_archive_password(password);
        ensure_file(&archive_path)?;
        ensure_supported_extract_format(&archive_path)?;

        let items = normalize_archive_promise_drag_items(items)?;
        macos_file_promise_drag::start_archive_entries_promise_drag(
            &window,
            archive_path,
            items,
            password,
        )
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, path, items, password);
        Err("当前仅支持在 macOS 上拖出压缩包内项目。".to_string())
    }
}

fn normalize_archive_promise_drag_items(
    items: Vec<ArchivePromiseDragItem>,
) -> Result<Vec<ArchivePromiseDragItem>, String> {
    if items.is_empty() {
        return Err("请选择要拖出的项目。".to_string());
    }

    let mut normalized = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let entry_path = item.entry_path.trim().trim_end_matches('/').to_string();
        if entry_path.is_empty() {
            return Err("请选择要拖出的项目。".to_string());
        }
        if archive_entry_has_unsafe_path(&entry_path) {
            return Err("该条目路径不安全，已阻止拖出。".to_string());
        }
        if !seen.insert((entry_path.clone(), item.is_dir)) {
            continue;
        }

        normalized.push(ArchivePromiseDragItem {
            promised_name: promised_archive_entry_name(&entry_path, &item.promised_name),
            entry_path,
            is_dir: item.is_dir,
        });
    }

    if normalized.is_empty() {
        Err("请选择要拖出的项目。".to_string())
    } else {
        Ok(normalized)
    }
}

fn extract_archive_items(
    path: String,
    output_dir: String,
    entries: Vec<String>,
    password: Option<String>,
) -> Result<OperationResult, String> {
    let archive_path = normalize_archive_open_path(&PathBuf::from(path))?;
    let password = normalize_archive_password(password);
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;

    let output_path = PathBuf::from(output_dir.trim());
    if output_path.as_os_str().is_empty() {
        return Err("请选择解压位置。".to_string());
    }
    fs::create_dir_all(&output_path).map_err(|err| format!("无法创建解压目录：{err}"))?;

    let _ = extract_task_entries(&archive_path, entries.clone(), password.as_deref())?;
    extract_entries_to_dir(&archive_path, &output_path, &entries, password.as_deref())?;

    let message = if entries.is_empty() {
        "解压完成。".to_string()
    } else {
        format!("已解压 {} 个项目。", entries.len())
    };

    Ok(OperationResult {
        output_path: output_path.to_string_lossy().to_string(),
        message,
    })
}

impl ExtractTask {
    fn new(total: usize, total_bytes: u64, output_path: String) -> Self {
        Self {
            state: Mutex::new(ExtractTaskState {
                status: "running".to_string(),
                total,
                completed: 0,
                total_bytes,
                completed_bytes: 0,
                current_bytes: 0,
                current_total_bytes: 0,
                current_item: String::new(),
                output_path,
                message: "正在准备解压。".to_string(),
                error: None,
                cancel_requested: false,
                pause_requested: false,
                child_pid: None,
            }),
        }
    }
}

fn prepare_batch_extract_plans(
    paths: Vec<String>,
    output_dir: &Path,
    password: Option<&str>,
) -> Result<(Vec<BatchExtractPlan>, usize, u64), String> {
    let archive_paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect();

    if archive_paths.is_empty() {
        return Err("请选择要解压的压缩包。".to_string());
    }

    let mut reserved_dirs = BTreeSet::new();
    let mut plans = Vec::new();
    let mut total = 0usize;
    let mut total_bytes = 0u64;

    for archive_path in archive_paths {
        let archive_path = normalize_archive_open_path(&archive_path)?;
        ensure_file(&archive_path)?;
        ensure_supported_extract_format(&archive_path)?;

        let output_name = archive_output_folder_name(&archive_path);
        let archive_output_dir =
            unique_directory_path(output_dir, &output_name, &mut reserved_dirs);
        let archive_label = file_name(&archive_path);
        let mut entries = extract_task_entries(&archive_path, Vec::new(), password)?;
        for entry in &mut entries {
            entry.display_path = format!("{archive_label} / {}", entry.display_path);
        }
        total = total.saturating_add(entries.len().max(1));
        total_bytes =
            total_bytes.saturating_add(entries.iter().map(|entry| entry.size).sum::<u64>().max(1));
        plans.push(BatchExtractPlan {
            archive_path,
            output_dir: archive_output_dir,
            entries,
        });
    }

    Ok((plans, total.max(1), total_bytes.max(1)))
}

fn archive_output_folder_name(path: &Path) -> String {
    let name = file_name(path);
    let lower = name.to_ascii_lowercase();
    if rar_part_number(&lower).is_some() {
        if let Some((base, _)) = lower.rsplit_once(".part") {
            return sanitize_path_segment(&name[..base.len()]);
        }
    }
    if let Some((base, _)) = numeric_split_volume_number(&lower) {
        return archive_output_folder_name(Path::new(&name[..base.len()]));
    }

    for extension in [
        ".tar.gz",
        ".tar.gzip",
        ".tar.bz2",
        ".tar.bzip2",
        ".tar.xz",
        ".tar.zst",
        ".tar.zstd",
        ".tar.lzma2",
        ".tar.lzma",
        ".tar.lz4",
        ".tgz",
        ".tbz2",
        ".tbz",
        ".txz",
        ".tzst",
        ".tlzma",
        ".tlz4",
        ".zip",
        ".rar",
        ".7z",
        ".tar",
        ".gzip",
        ".bzip2",
        ".gz",
        ".bz2",
        ".xz",
        ".lzh",
        ".lha",
        ".zstd",
        ".zst",
        ".lzma2",
        ".lzma",
        ".lz4",
        ".iso",
        ".z",
    ] {
        if lower.ends_with(extension) {
            let trimmed = &name[..name.len().saturating_sub(extension.len())];
            return sanitize_path_segment(trimmed);
        }
    }
    sanitize_path_segment(
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&name),
    )
}

fn unique_directory_path(
    output_dir: &Path,
    folder_name: &str,
    reserved_dirs: &mut BTreeSet<PathBuf>,
) -> PathBuf {
    let safe_name = sanitize_path_segment(folder_name);
    let mut candidate = output_dir.join(&safe_name);
    let mut index = 2;
    while candidate.exists() || reserved_dirs.contains(&candidate) {
        candidate = output_dir.join(format!("{safe_name} {index}"));
        index += 1;
    }
    reserved_dirs.insert(candidate.clone());
    candidate
}

fn extract_task_entries(
    archive_path: &Path,
    entries: Vec<String>,
    password: Option<&str>,
) -> Result<Vec<ExtractTaskEntry>, String> {
    let requested: Vec<String> = entries
        .into_iter()
        .filter(|entry| !entry.trim().is_empty())
        .collect();

    if is_single_stream_format(archive_path) {
        let metadata = fs::metadata(archive_path).map_err(|err| err.to_string())?;
        let output_name = single_stream_output_name(archive_path);
        return Ok(vec![ExtractTaskEntry {
            path: output_name.clone(),
            display_path: output_name,
            size: metadata.len().max(1),
            is_dir: false,
            is_unsafe_path: false,
        }]);
    }

    let listing_entries = list_archive_entries_for_task(archive_path, password)?;

    if !requested.is_empty() {
        let selected_entries: Vec<ExtractTaskEntry> = requested
            .into_iter()
            .map(|path| {
                let listing_entry = listing_entries.iter().find(|entry| entry.path == path);
                let size = listing_entry.map(|entry| entry.size).unwrap_or(1);
                let is_dir = listing_entry
                    .map(|entry| entry.is_dir)
                    .unwrap_or_else(|| path.ends_with('/'));
                ExtractTaskEntry {
                    display_path: strip_archive_entry_prefix(&path)
                        .to_string_lossy()
                        .to_string(),
                    is_unsafe_path: archive_entry_has_unsafe_path(&path),
                    path,
                    size: size.max(1),
                    is_dir,
                }
            })
            .collect();
        ensure_safe_archive_entry_paths(&selected_entries)?;
        return Ok(selected_entries);
    }

    if listing_entries.is_empty() {
        Ok(vec![ExtractTaskEntry {
            path: file_name(archive_path),
            display_path: file_name(archive_path),
            size: fs::metadata(archive_path)
                .map(|metadata| metadata.len().max(1))
                .unwrap_or(1),
            is_dir: false,
            is_unsafe_path: false,
        }])
    } else {
        ensure_safe_archive_entry_paths(&listing_entries)?;
        Ok(listing_entries)
    }
}

fn list_archive_entries_for_task(
    archive_path: &Path,
    password: Option<&str>,
) -> Result<Vec<ExtractTaskEntry>, String> {
    let prepared_archive = prepare_archive_for_read(archive_path)?;
    Ok(
        read_archive_listing_entries(prepared_archive.path(), password)?
            .into_iter()
            .map(|entry| ExtractTaskEntry {
                display_path: strip_archive_entry_prefix(&entry.path)
                    .to_string_lossy()
                    .to_string(),
                path: entry.path,
                size: if entry.kind == "folder" {
                    1
                } else {
                    entry.size.max(1)
                },
                is_dir: entry.kind == "folder",
                is_unsafe_path: entry.is_unsafe_path,
            })
            .collect(),
    )
}

fn run_extract_task(
    task_id: String,
    task: Arc<ExtractTask>,
    archive_path: PathBuf,
    output_dir: PathBuf,
    entries: Vec<ExtractTaskEntry>,
    password: Option<String>,
    conflict_strategy: ExtractConflictStrategy,
) {
    let result = if is_single_stream_format(&archive_path) {
        run_single_stream_extract_task(
            &task_id,
            &task,
            &archive_path,
            &output_dir,
            conflict_strategy,
        )
    } else {
        run_archive_extract_task(
            &task_id,
            &task,
            &archive_path,
            &output_dir,
            &entries,
            password.as_deref(),
            conflict_strategy,
        )
    };

    match result {
        Ok(()) => {
            if let Ok(mut state) = task.state.lock() {
                if state.status != "canceled" {
                    state.status = "completed".to_string();
                    state.completed = state.total;
                    state.completed_bytes = state.total_bytes;
                    state.current_bytes = 0;
                    state.current_total_bytes = 0;
                    state.current_item.clear();
                    state.child_pid = None;
                    state.message = format!("解压完成，已保存到：{}", state.output_path);
                }
            }
        }
        Err(ExtractTaskError::Canceled) => {
            if let Ok(mut state) = task.state.lock() {
                state.status = "canceled".to_string();
                state.child_pid = None;
                state.message = "解压已取消。".to_string();
            }
        }
        Err(ExtractTaskError::Failed(error)) => {
            if let Ok(mut state) = task.state.lock() {
                state.status = "failed".to_string();
                state.child_pid = None;
                state.error = Some(error.clone());
                state.message = error;
            }
        }
    }
}

fn run_batch_extract_task(
    task_id: String,
    task: Arc<ExtractTask>,
    plans: Vec<BatchExtractPlan>,
    password: Option<String>,
    conflict_strategy: ExtractConflictStrategy,
) {
    let result = (|| -> Result<(), ExtractTaskError> {
        for plan in plans {
            wait_for_extract_task(&task)?;
            fs::create_dir_all(&plan.output_dir)
                .map_err(|err| ExtractTaskError::Failed(format!("无法创建解压目录：{err}")))?;
            {
                let mut state = task_state(&task)?;
                state.status = "running".to_string();
                state.message = format!("正在解压 {}", file_name(&plan.archive_path));
            }

            if is_single_stream_format(&plan.archive_path) {
                run_single_stream_extract_task(
                    &task_id,
                    &task,
                    &plan.archive_path,
                    &plan.output_dir,
                    conflict_strategy,
                )?;
            } else {
                run_archive_extract_task(
                    &task_id,
                    &task,
                    &plan.archive_path,
                    &plan.output_dir,
                    &plan.entries,
                    password.as_deref(),
                    conflict_strategy,
                )?;
            }
        }
        Ok(())
    })();

    match result {
        Ok(()) => {
            if let Ok(mut state) = task.state.lock() {
                if state.status != "canceled" {
                    state.status = "completed".to_string();
                    state.completed = state.total;
                    state.completed_bytes = state.total_bytes;
                    state.current_bytes = 0;
                    state.current_total_bytes = 0;
                    state.current_item.clear();
                    state.child_pid = None;
                    state.message = format!("批量解压完成，已保存到：{}", state.output_path);
                }
            }
        }
        Err(ExtractTaskError::Canceled) => {
            if let Ok(mut state) = task.state.lock() {
                state.status = "canceled".to_string();
                state.child_pid = None;
                state.message = "解压已取消。".to_string();
            }
        }
        Err(ExtractTaskError::Failed(error)) => {
            if let Ok(mut state) = task.state.lock() {
                state.status = "failed".to_string();
                state.child_pid = None;
                state.error = Some(error.clone());
                state.message = error;
            }
        }
    }
}

fn run_archive_extract_task(
    task_id: &str,
    task: &Arc<ExtractTask>,
    archive_path: &Path,
    output_dir: &Path,
    entries: &[ExtractTaskEntry],
    password: Option<&str>,
    conflict_strategy: ExtractConflictStrategy,
) -> Result<(), ExtractTaskError> {
    let prepared_archive =
        prepare_archive_for_read(archive_path).map_err(ExtractTaskError::Failed)?;
    for (index, entry) in entries.iter().enumerate() {
        wait_for_extract_task(task)?;
        let expected_output_path = output_dir.join(strip_archive_entry_prefix(&entry.path));
        {
            let mut state = task_state(task)?;
            state.status = "running".to_string();
            state.current_item = entry.display_path.clone();
            state.current_bytes = 0;
            state.current_total_bytes = entry.size.max(1);
            state.message = format!(
                "正在解压 {} / {}",
                state.completed.saturating_add(1),
                state.total
            );
        }

        if conflict_strategy == ExtractConflictStrategy::Skip && expected_output_path.exists() {
            complete_extract_task_entry(task, format!("已跳过已存在项目：{}", entry.display_path))?;
            continue;
        }

        let mut temp_extract_dir = None;
        let mut monitor_path = expected_output_path.clone();
        let extract_dir = if conflict_strategy == ExtractConflictStrategy::Rename
            && !entry.is_dir
            && expected_output_path.exists()
        {
            let temp_dir = std::env::temp_dir()
                .join("cardiganzip-conflicts")
                .join(sanitize_path_segment(task_id))
                .join(index.to_string());
            let _ = fs::remove_dir_all(&temp_dir);
            fs::create_dir_all(&temp_dir)
                .map_err(|err| ExtractTaskError::Failed(format!("无法创建临时解压目录：{err}")))?;
            monitor_path = temp_dir.join(strip_archive_entry_prefix(&entry.path));
            temp_extract_dir = Some(temp_dir.clone());
            temp_dir
        } else {
            output_dir.to_path_buf()
        };

        let mut args = Vec::new();
        append_archive_extract_password_args(&mut args, password);
        args.push(OsString::from("-xf"));
        args.push(prepared_archive.path().as_os_str().to_os_string());
        args.push(OsString::from("-C"));
        args.push(extract_dir.as_os_str().to_os_string());
        args.push(OsString::from(&entry.path));
        run_extract_child(
            task_id,
            task,
            "bsdtar",
            args,
            Some(monitor_path.as_path()),
            None,
        )?;

        if let Some(temp_dir) = temp_extract_dir {
            move_conflicting_extract_output(&monitor_path, &expected_output_path)?;
            let _ = fs::remove_dir_all(temp_dir);
        }

        complete_extract_task_entry(task, String::new())?;
    }

    Ok(())
}

fn run_single_stream_extract_task(
    task_id: &str,
    task: &Arc<ExtractTask>,
    archive_path: &Path,
    output_dir: &Path,
    conflict_strategy: ExtractConflictStrategy,
) -> Result<(), ExtractTaskError> {
    let (program, args) =
        single_stream_decompress_command(archive_path).map_err(ExtractTaskError::Failed)?;

    wait_for_extract_task(task)?;
    let output_name = single_stream_output_name(archive_path);
    let desired_output_path = output_dir.join(&output_name);
    let output_path = if conflict_strategy == ExtractConflictStrategy::Rename {
        unique_file_path(output_dir, &output_name)
    } else {
        desired_output_path.clone()
    };
    {
        let mut state = task_state(task)?;
        state.status = "running".to_string();
        state.current_item = output_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        state.current_bytes = 0;
        state.current_total_bytes = fs::metadata(archive_path)
            .map(|metadata| metadata.len().max(1))
            .unwrap_or(1);
        state.message = "正在解压单文件压缩流。".to_string();
    }

    if conflict_strategy == ExtractConflictStrategy::Skip && desired_output_path.exists() {
        complete_extract_task_entry(task, format!("已跳过已存在文件：{output_name}"))?;
        return Ok(());
    }

    run_extract_child_to_file(task_id, task, program, args, &output_path, None)?;

    complete_extract_task_entry(task, String::new())?;
    Ok(())
}

fn complete_extract_task_entry(
    task: &Arc<ExtractTask>,
    message: String,
) -> Result<(), ExtractTaskError> {
    let mut state = task_state(task)?;
    state.completed = state.completed.saturating_add(1).min(state.total);
    state.completed_bytes = state
        .completed_bytes
        .saturating_add(state.current_total_bytes)
        .min(state.total_bytes);
    state.current_bytes = 0;
    if !message.is_empty() {
        state.message = message;
    }
    Ok(())
}

fn move_conflicting_extract_output(
    source: &Path,
    expected_output_path: &Path,
) -> Result<(), ExtractTaskError> {
    if !source.exists() {
        return Err(ExtractTaskError::Failed(
            "无法找到临时解压结果。".to_string(),
        ));
    }

    let target_parent = expected_output_path
        .parent()
        .unwrap_or_else(|| Path::new(""));
    fs::create_dir_all(target_parent)
        .map_err(|err| ExtractTaskError::Failed(format!("无法创建目标目录：{err}")))?;
    let target_name = expected_output_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名");
    let target_path = unique_file_path(target_parent, target_name);
    fs::rename(source, &target_path)
        .map_err(|err| ExtractTaskError::Failed(format!("无法移动重命名文件：{err}")))?;
    Ok(())
}

fn wait_for_extract_task(task: &Arc<ExtractTask>) -> Result<(), ExtractTaskError> {
    loop {
        {
            let state = task_state(task)?;
            if state.cancel_requested {
                return Err(ExtractTaskError::Canceled);
            }
            if !state.pause_requested {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(120));
    }
}

fn run_extract_child(
    task_id: &str,
    task: &Arc<ExtractTask>,
    program: &str,
    args: Vec<OsString>,
    monitor_path: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<Vec<u8>, ExtractTaskError> {
    let candidates = command_candidates(program);
    let mut last_not_found = false;

    for (index, candidate) in candidates.iter().enumerate() {
        let mut command = Command::new(candidate);
        command.args(&args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::null());
        command.stderr(Stdio::piped());
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }

        match command.spawn() {
            Ok(child) => {
                let pid = child.id();
                {
                    let mut state = task_state(task)?;
                    state.child_pid = Some(pid);
                    if state.pause_requested {
                        let _ = send_process_signal(pid, "STOP");
                    }
                }

                let result = wait_for_child(task_id, task, child, program, monitor_path);
                if let Ok(mut state) = task.state.lock() {
                    state.child_pid = None;
                }
                return result;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && index + 1 < candidates.len() =>
            {
                last_not_found = true;
            }
            Err(_) => {
                return Err(ExtractTaskError::Failed(format!(
                    "找不到命令：{program}。请确认系统已安装对应压缩工具。"
                )));
            }
        }
    }

    if last_not_found {
        Err(ExtractTaskError::Failed(format!(
            "找不到命令：{program}。请确认系统已安装对应压缩工具。"
        )))
    } else {
        Err(ExtractTaskError::Failed(format!("{program} 执行失败。")))
    }
}

fn run_extract_child_to_file(
    task_id: &str,
    task: &Arc<ExtractTask>,
    program: &str,
    args: Vec<OsString>,
    output_path: &Path,
    current_dir: Option<&Path>,
) -> Result<(), ExtractTaskError> {
    let candidates = command_candidates(program);

    for (index, candidate) in candidates.iter().enumerate() {
        let output_file = File::create(output_path)
            .map_err(|err| ExtractTaskError::Failed(format!("无法创建解压文件：{err}")))?;
        let mut command = Command::new(candidate);
        command.args(&args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::from(output_file));
        command.stderr(Stdio::piped());
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }

        match command.spawn() {
            Ok(child) => {
                let pid = child.id();
                {
                    let mut state = task_state(task)?;
                    state.child_pid = Some(pid);
                    if state.pause_requested {
                        let _ = send_process_signal(pid, "STOP");
                    }
                }

                let result =
                    wait_for_child(task_id, task, child, program, Some(output_path)).map(|_| ());
                if let Ok(mut state) = task.state.lock() {
                    state.child_pid = None;
                }
                return result;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && index + 1 < candidates.len() => {
            }
            Err(_) => {
                return Err(ExtractTaskError::Failed(format!(
                    "找不到命令：{program}。请确认系统已安装对应压缩工具。"
                )));
            }
        }
    }

    Err(ExtractTaskError::Failed(format!(
        "找不到命令：{program}。请确认系统已安装对应压缩工具。"
    )))
}

fn wait_for_child(
    task_id: &str,
    task: &Arc<ExtractTask>,
    mut child: Child,
    program: &str,
    monitor_path: Option<&Path>,
) -> Result<Vec<u8>, ExtractTaskError> {
    loop {
        {
            let mut state = task_state(task)?;
            if state.cancel_requested {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExtractTaskError::Canceled);
            }
            if let Some(path) = monitor_path {
                state.current_bytes = monitored_path_size(path)
                    .unwrap_or(state.current_bytes)
                    .min(state.current_total_bytes);
            }
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                return collect_child_output(child, status.success(), program);
            }
            Ok(None) => {
                thread::sleep(Duration::from_millis(120));
            }
            Err(error) => {
                return Err(ExtractTaskError::Failed(format!(
                    "解压任务 {task_id} 状态读取失败：{error}"
                )));
            }
        }
    }
}

fn collect_child_output(
    mut child: Child,
    success: bool,
    program: &str,
) -> Result<Vec<u8>, ExtractTaskError> {
    let mut stdout = Vec::new();
    let mut stderr = String::new();

    if let Some(mut child_stdout) = child.stdout.take() {
        child_stdout
            .read_to_end(&mut stdout)
            .map_err(|err| ExtractTaskError::Failed(format!("读取解压输出失败：{err}")))?;
    }
    if let Some(mut child_stderr) = child.stderr.take() {
        child_stderr
            .read_to_string(&mut stderr)
            .map_err(|err| ExtractTaskError::Failed(format!("读取解压错误信息失败：{err}")))?;
    }

    if success {
        Ok(stdout)
    } else {
        let stderr = stderr.trim();
        Err(ExtractTaskError::Failed(if stderr.is_empty() {
            format!("{program} 执行失败。")
        } else {
            stderr.to_string()
        }))
    }
}

fn task_state(
    task: &Arc<ExtractTask>,
) -> Result<std::sync::MutexGuard<'_, ExtractTaskState>, ExtractTaskError> {
    task.state
        .lock()
        .map_err(|_| ExtractTaskError::Failed("无法读取解压任务状态。".to_string()))
}

fn find_extract_task(
    tasks: &tauri::State<'_, ExtractTasks>,
    task_id: &str,
) -> Result<Arc<ExtractTask>, String> {
    tasks
        .tasks
        .lock()
        .map_err(|_| "无法读取解压任务。".to_string())?
        .get(task_id)
        .cloned()
        .ok_or_else(|| "找不到解压任务。".to_string())
}

fn find_compress_task(
    tasks: &tauri::State<'_, CompressTasks>,
    task_id: &str,
) -> Result<Arc<ExtractTask>, String> {
    tasks
        .tasks
        .lock()
        .map_err(|_| "无法读取压缩任务。".to_string())?
        .get(task_id)
        .cloned()
        .ok_or_else(|| "找不到压缩任务。".to_string())
}

fn get_extract_task_progress(
    tasks: &tauri::State<'_, ExtractTasks>,
    task_id: &str,
) -> Result<ExtractTaskProgress, String> {
    let task = find_extract_task(tasks, task_id)?;
    let state = task
        .state
        .lock()
        .map_err(|_| "无法读取解压进度。".to_string())?;
    Ok(extract_task_progress(task_id, &state))
}

fn get_compress_task_progress(
    tasks: &tauri::State<'_, CompressTasks>,
    task_id: &str,
) -> Result<ExtractTaskProgress, String> {
    let task = find_compress_task(tasks, task_id)?;
    let state = task
        .state
        .lock()
        .map_err(|_| "无法读取压缩进度。".to_string())?;
    Ok(extract_task_progress(task_id, &state))
}

fn extract_task_progress(task_id: &str, state: &ExtractTaskState) -> ExtractTaskProgress {
    let completed_bytes = state
        .completed_bytes
        .saturating_add(state.current_bytes)
        .min(state.total_bytes);

    ExtractTaskProgress {
        task_id: task_id.to_string(),
        status: state.status.clone(),
        total: state.total,
        completed: state.completed,
        total_bytes: state.total_bytes,
        completed_bytes,
        current_bytes: state.current_bytes,
        current_total_bytes: state.current_total_bytes,
        current_item: state.current_item.clone(),
        output_path: state.output_path.clone(),
        message: state.message.clone(),
        error: state.error.clone(),
    }
}

fn monitored_path_size(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.is_dir() {
        Some(1)
    } else {
        Some(metadata.len())
    }
}

fn send_process_signal(pid: u32, signal: &str) -> Result<(), String> {
    let status = Command::new("/bin/kill")
        .arg(format!("-{signal}"))
        .arg(pid.to_string())
        .status()
        .map_err(|err| format!("无法发送进程信号：{err}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("进程信号发送失败。".to_string())
    }
}

#[tauri::command]
fn default_output_dir(path: Option<String>) -> String {
    if let Some(path) = path {
        let candidate = PathBuf::from(path);
        if candidate.is_dir() {
            return candidate.to_string_lossy().to_string();
        }
        if let Some(parent) = candidate.parent() {
            return parent.to_string_lossy().to_string();
        }
    }

    std::env::var("HOME")
        .map(|home| format!("{home}/Desktop"))
        .unwrap_or_else(|_| ".".to_string())
}

fn describe_path(path: &Path) -> Result<FileInfo, String> {
    let metadata = fs::metadata(path).map_err(|err| format!("无法读取文件信息：{err}"))?;
    Ok(FileInfo {
        path: path.to_string_lossy().to_string(),
        name: file_name(path),
        kind: if metadata.is_dir() {
            "folder".to_string()
        } else {
            file_kind(path)
        },
        size: if metadata.is_file() {
            metadata.len()
        } else {
            directory_size(path)?
        },
        size_label: format_size(if metadata.is_file() {
            metadata.len()
        } else {
            directory_size(path)?
        }),
        modified_label: metadata
            .modified()
            .ok()
            .map(format_system_time)
            .unwrap_or_else(|| "-".to_string()),
    })
}

fn describe_compress_path(path: &Path) -> Result<FileInfo, String> {
    let metadata = fs::metadata(path).map_err(|err| format!("无法读取文件信息：{err}"))?;
    let is_dir = metadata.is_dir();
    let size = if metadata.is_file() {
        metadata.len()
    } else {
        0
    };
    Ok(FileInfo {
        path: path.to_string_lossy().to_string(),
        name: file_name(path),
        kind: if is_dir {
            "folder".to_string()
        } else {
            file_kind(path)
        },
        size,
        size_label: if is_dir {
            "-".to_string()
        } else {
            format_size(size)
        },
        modified_label: metadata
            .modified()
            .ok()
            .map(format_system_time)
            .unwrap_or_else(|| "-".to_string()),
    })
}

fn directory_size(path: &Path) -> Result<u64, String> {
    let mut total = 0;
    for entry in fs::read_dir(path).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let metadata = entry.metadata().map_err(|err| err.to_string())?;
        if metadata.is_dir() {
            total += directory_size(&entry.path())?;
        } else {
            total += metadata.len();
        }
    }
    Ok(total)
}

fn read_archive_listing_entries(
    archive_path: &Path,
    password: Option<&str>,
) -> Result<Vec<ArchiveEntry>, String> {
    let mut path_args = Vec::new();
    append_archive_password_args(&mut path_args, password);
    path_args.push(OsString::from("-tf"));
    path_args.push(archive_path.as_os_str().to_os_string());
    let path_output = run_command("bsdtar", path_args.iter().map(|arg| arg.as_os_str()), None)?;

    let mut detail_args = Vec::new();
    append_archive_password_args(&mut detail_args, password);
    detail_args.push(OsString::from("-tvf"));
    detail_args.push(archive_path.as_os_str().to_os_string());
    let detail_output = run_command(
        "bsdtar",
        detail_args.iter().map(|arg| arg.as_os_str()),
        None,
    )
    .unwrap_or_default();

    Ok(parse_archive_listing(&path_output, &detail_output))
}

#[derive(Debug, Clone)]
struct ArchiveListingDetail {
    permissions: String,
    size: u64,
    modified_label: String,
}

fn parse_archive_listing_detail(line: &str) -> Option<ArchiveListingDetail> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    let permissions = parts.first()?.to_string();
    let Some(date_index) = parts.iter().position(|part| is_archive_month(part)) else {
        return Some(ArchiveListingDetail {
            permissions,
            size: 0,
            modified_label: "-".to_string(),
        });
    };
    if date_index < 1 || parts.len() <= date_index + 2 {
        return Some(ArchiveListingDetail {
            permissions,
            size: 0,
            modified_label: "-".to_string(),
        });
    }

    Some(ArchiveListingDetail {
        permissions,
        size: parts[date_index - 1].parse::<u64>().unwrap_or(0),
        modified_label: archive_listing_time_label(
            parts[date_index],
            parts[date_index + 1],
            parts[date_index + 2],
        ),
    })
}

fn parse_archive_listing(path_output: &str, detail_output: &str) -> Vec<ArchiveEntry> {
    let details: Vec<Option<ArchiveListingDetail>> = detail_output
        .lines()
        .map(parse_archive_listing_detail)
        .collect();

    path_output
        .lines()
        .enumerate()
        .filter_map(|(index, raw_path)| {
            let path = decode_archive_listing_path(raw_path.trim_end_matches('\r'));
            if path.trim().is_empty() {
                return None;
            }

            let detail = details.get(index).and_then(|detail| detail.as_ref());
            let permissions = detail
                .map(|detail| detail.permissions.as_str())
                .unwrap_or("");
            let display_path = path.trim_end_matches('/').to_string();
            let name = Path::new(&display_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&display_path)
                .to_string();
            let is_folder = permissions.starts_with('d') || path.ends_with('/');
            let kind = if is_folder {
                "folder".to_string()
            } else {
                file_kind(Path::new(&name))
            };
            let is_hidden = archive_entry_is_hidden(&display_path);
            let is_executable =
                !is_folder && archive_entry_is_executable(&display_path, permissions);
            let is_unsafe_path = archive_entry_has_unsafe_path(&path);
            let size = detail.map(|detail| detail.size).unwrap_or(0);
            let modified_label = detail
                .map(|detail| detail.modified_label.clone())
                .unwrap_or_else(|| "-".to_string());

            let type_label = if is_folder {
                "文件夹".to_string()
            } else {
                format!("{} 文件", extension_label(Path::new(&name)))
            };

            Some(ArchiveEntry {
                name,
                path,
                kind: kind.clone(),
                type_label,
                size,
                size_label: if is_folder {
                    "-".to_string()
                } else {
                    format_size(size)
                },
                modified_label,
                crc: None,
                method: None,
                is_encrypted: false,
                is_hidden,
                is_executable,
                is_unsafe_path,
            })
        })
        .collect()
}

fn decode_archive_listing_path(path: &str) -> String {
    let mut bytes = Vec::with_capacity(path.len());
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let mut octal = String::new();
            for _ in 0..3 {
                let Some(next) = chars.peek().copied() else {
                    break;
                };
                if !matches!(next, '0'..='7') {
                    break;
                }
                octal.push(next);
                chars.next();
            }

            if octal.len() == 3 {
                if let Ok(value) = u8::from_str_radix(&octal, 8) {
                    bytes.push(value);
                    continue;
                }
            }

            bytes.extend_from_slice(b"\\");
            bytes.extend_from_slice(octal.as_bytes());
            continue;
        }

        let mut buffer = [0; 4];
        bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
    }

    String::from_utf8_lossy(&bytes).into_owned()
}

#[derive(Default)]
struct ArchiveEntryDetail {
    crc: Option<String>,
    method: Option<String>,
    is_encrypted: bool,
}

fn apply_archive_entry_details(archive_path: &Path, entries: &mut [ArchiveEntry]) {
    if archive_format(archive_path) != "ZIP" {
        return;
    }
    let details = zip_archive_entry_details(archive_path);
    if details.is_empty() {
        return;
    }

    for entry in entries {
        let key = entry.path.trim_end_matches('/');
        if let Some(detail) = details.get(key) {
            entry.crc = detail.crc.clone();
            entry.method = detail.method.clone();
            entry.is_encrypted = detail.is_encrypted;
        }
    }
}

fn zip_archive_entry_details(archive_path: &Path) -> HashMap<String, ArchiveEntryDetail> {
    let output = run_command(
        "zipinfo",
        [OsStr::new("-v"), archive_path.as_os_str()].into_iter(),
        None,
    )
    .or_else(|_| {
        run_command(
            "unzip",
            [OsStr::new("-Z"), OsStr::new("-v"), archive_path.as_os_str()].into_iter(),
            None,
        )
    });
    let Ok(output) = output else {
        return HashMap::new();
    };

    let mut details = HashMap::new();
    let mut current_name: Option<String> = None;
    let mut waiting_for_name = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Central directory entry #") {
            waiting_for_name = true;
            current_name = None;
            continue;
        }
        if waiting_for_name {
            if trimmed.is_empty() || trimmed.starts_with('-') {
                continue;
            }
            current_name = Some(trimmed.trim_end_matches('/').to_string());
            details.entry(current_name.clone().unwrap()).or_default();
            waiting_for_name = false;
            continue;
        }

        let Some(name) = current_name.as_ref() else {
            continue;
        };
        let Some((label, value)) = trimmed.split_once(':') else {
            continue;
        };
        let detail: &mut ArchiveEntryDetail = details.entry(name.clone()).or_default();
        match label.trim() {
            "compression method" => detail.method = Some(value.trim().to_string()),
            "file security status" => {
                detail.is_encrypted = value.trim().eq_ignore_ascii_case("encrypted")
            }
            "32-bit CRC value (hex)" => detail.crc = Some(value.trim().to_ascii_uppercase()),
            _ => {}
        }
    }

    details
}

fn detect_archive_encryption(
    archive_path: &Path,
    entries: &[ArchiveEntry],
    password: Option<&str>,
) -> Result<bool, String> {
    if entries.iter().any(|entry| entry.is_encrypted) {
        return Ok(true);
    }
    if password.is_some() && archive_format(archive_path) == "7Z" {
        return Ok(true);
    }

    let Some(first_file) = entries.iter().find(|entry| entry.kind != "folder") else {
        return Ok(false);
    };
    if is_single_stream_format(archive_path) {
        return Ok(false);
    }

    let mut args = Vec::new();
    append_archive_extract_password_args(&mut args, password);
    args.push(OsString::from("-xOf"));
    args.push(archive_path.as_os_str().to_os_string());
    args.push(OsString::from(&first_file.path));

    match run_command_bytes("bsdtar", args.iter().map(|arg| arg.as_os_str()), None) {
        Ok(_) => Ok(false),
        Err(error) => Ok(is_archive_password_error_message(&error)),
    }
}

fn is_archive_password_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("password")
        || lower.contains("passphrase")
        || lower.contains("decrypt")
        || lower.contains("encrypted")
        || message.contains("密码")
        || message.contains("口令")
}

fn archive_properties(
    archive_path: &Path,
    entries: &[ArchiveEntry],
    archive_size: u64,
    is_encrypted: bool,
) -> ArchiveProperties {
    let uncompressed_size = entries
        .iter()
        .filter(|entry| entry.kind != "folder")
        .map(|entry| entry.size)
        .sum::<u64>();
    let compression_ratio_label = if uncompressed_size == 0 {
        "-".to_string()
    } else {
        format!(
            "{:.1}%",
            archive_size as f64 * 100.0 / uncompressed_size as f64
        )
    };
    let mut methods = entries
        .iter()
        .filter_map(|entry| entry.method.as_deref())
        .filter(|method| !method.trim().is_empty())
        .collect::<BTreeSet<_>>();
    let method_summary = if methods.is_empty() {
        archive_format(archive_path)
    } else if methods.len() <= 2 {
        methods.into_iter().collect::<Vec<_>>().join("、")
    } else {
        let first = methods.pop_first().unwrap_or("");
        format!("{first} 等 {} 种", methods.len() + 1)
    };
    let encrypted_count = entries
        .iter()
        .filter(|entry| entry.kind != "folder" && entry.is_encrypted)
        .count();

    ArchiveProperties {
        uncompressed_size,
        uncompressed_size_label: format_size(uncompressed_size),
        compression_ratio_label,
        method_summary,
        crc_available: entries.iter().any(|entry| entry.crc.is_some()),
        is_encrypted,
        encrypted_count,
        split: archive_split_summary(archive_path),
    }
}

fn archive_split_summary(archive_path: &Path) -> ArchiveSplitSummary {
    let volumes = archive_split_volumes(archive_path);
    let total_size = volumes
        .iter()
        .filter_map(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
        .sum::<u64>();
    let first_volume = volumes
        .first()
        .map_or(archive_path, |path| path.as_path())
        .to_string_lossy()
        .to_string();

    ArchiveSplitSummary {
        is_split: volumes.len() > 1 || split_archive_format(archive_path).is_some(),
        volume_count: volumes.len().max(1),
        total_size: total_size.max(
            fs::metadata(archive_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        ),
        total_size_label: format_size(
            total_size.max(
                fs::metadata(archive_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0),
            ),
        ),
        first_volume,
    }
}

fn archive_split_volumes(archive_path: &Path) -> Vec<PathBuf> {
    let parent = archive_path.parent().unwrap_or_else(|| Path::new(""));
    let name = file_name(archive_path);
    let lower = name.to_ascii_lowercase();

    if let Some((base, _)) = numeric_split_volume_number(&lower) {
        return numeric_split_siblings(parent, base);
    }

    if archive_format(archive_path) == "ZIP" {
        let stem = archive_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        let mut volumes = Vec::new();
        if let Ok(entries) = fs::read_dir(parent) {
            for entry in entries.flatten() {
                let path = entry.path();
                let matches_stem = path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|value| value == stem)
                    .unwrap_or(false);
                let is_zip_part = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|extension| {
                        let lower = extension.to_ascii_lowercase();
                        lower.len() == 3
                            && lower.starts_with('z')
                            && lower[1..].chars().all(|ch| ch.is_ascii_digit())
                    })
                    .unwrap_or(false);
                if matches_stem && is_zip_part {
                    volumes.push(path);
                }
            }
        }
        volumes.sort();
        if archive_path.exists() {
            volumes.push(archive_path.to_path_buf());
        }
        if !volumes.is_empty() {
            return volumes;
        }
    }

    vec![archive_path.to_path_buf()]
}

fn numeric_split_siblings(parent: &Path, base: &str) -> Vec<PathBuf> {
    let mut volumes = Vec::new();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let lower_name = name.to_ascii_lowercase();
            let Some((candidate_base, _)) = numeric_split_volume_number(&lower_name) else {
                continue;
            };
            if candidate_base == base {
                volumes.push(path);
            }
        }
    }
    volumes.sort();
    volumes
}

fn is_archive_month(value: &str) -> bool {
    matches!(
        value,
        "Jan"
            | "Feb"
            | "Mar"
            | "Apr"
            | "May"
            | "Jun"
            | "Jul"
            | "Aug"
            | "Sep"
            | "Oct"
            | "Nov"
            | "Dec"
    )
}

fn archive_listing_time_label(month: &str, day: &str, year_or_time: &str) -> String {
    let month = archive_month_number(month).unwrap_or(1);
    let day = day.parse::<u32>().unwrap_or(1);
    if let Some((hour, minute)) = parse_hour_minute(year_or_time) {
        let now = Local::now();
        return format!(
            "{}年{}月{}日 {:02}:{:02}",
            now.year(),
            month,
            day,
            hour,
            minute
        );
    }

    let year = year_or_time
        .parse::<i32>()
        .unwrap_or_else(|_| Local::now().year());
    format!("{year}年{month}月{day}日 00:00")
}

fn archive_month_number(value: &str) -> Option<u32> {
    Some(match value {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn parse_hour_minute(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.split_once(':')?;
    Some((hour.parse().ok()?, minute.parse().ok()?))
}

fn single_stream_entry(path: &Path) -> Result<ArchiveEntry, String> {
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    let output_name = single_stream_output_name(path);

    Ok(ArchiveEntry {
        name: output_name.clone(),
        path: output_name.clone(),
        kind: file_kind(Path::new(&output_name)),
        type_label: format!("{} 文件", extension_label(Path::new(&output_name))),
        size: metadata.len(),
        size_label: format_size(metadata.len()),
        modified_label: metadata
            .modified()
            .ok()
            .map(format_system_time)
            .unwrap_or_else(|| "-".to_string()),
        crc: None,
        method: Some(archive_format(path)),
        is_encrypted: false,
        is_hidden: archive_entry_is_hidden(&output_name),
        is_executable: archive_entry_is_executable(&output_name, ""),
        is_unsafe_path: archive_entry_has_unsafe_path(&output_name),
    })
}

fn extract_single_stream(archive_path: &Path, output_dir: &Path) -> Result<(), String> {
    let (program, args) = single_stream_decompress_command(archive_path)?;

    let output = run_command_bytes(program, args.iter().map(|arg| arg.as_os_str()), None)?;

    let output_path = unique_file_path(output_dir, &single_stream_output_name(archive_path));
    let mut file = File::create(&output_path).map_err(|err| format!("无法创建解压文件：{err}"))?;
    file.write_all(&output)
        .map_err(|err| format!("无法写入解压文件：{err}"))?;
    Ok(())
}

fn extract_entries_to_dir(
    archive_path: &Path,
    output_dir: &Path,
    entries: &[String],
    password: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    if is_single_stream_format(archive_path) {
        extract_single_stream(archive_path, output_dir)?;
        return Ok(vec![
            output_dir.join(single_stream_output_name(archive_path))
        ]);
    }

    let prepared_archive = prepare_archive_for_read(archive_path)?;
    let mut args = Vec::new();
    append_archive_extract_password_args(&mut args, password);
    args.push(OsString::from("-xf"));
    args.push(prepared_archive.path().as_os_str().to_os_string());
    args.push(OsString::from("-C"));
    args.push(output_dir.as_os_str().to_os_string());

    for entry in entries.iter().filter(|entry| !entry.trim().is_empty()) {
        args.push(OsString::from(entry));
    }

    run_command("bsdtar", args.iter().map(|arg| arg.as_os_str()), None)?;

    Ok(if entries.is_empty() {
        vec![output_dir.to_path_buf()]
    } else {
        entries
            .iter()
            .map(|entry| output_dir.join(strip_archive_entry_prefix(entry)))
            .collect()
    })
}

#[cfg(test)]
fn extract_archive_entry_to_promised_file(
    archive_path: &Path,
    entry_path: &str,
    promised_path: &Path,
    password: Option<&str>,
) -> Result<PathBuf, String> {
    extract_archive_entry_to_promised_item(archive_path, entry_path, promised_path, false, password)
}

fn extract_archive_entry_to_promised_item(
    archive_path: &Path,
    entry_path: &str,
    promised_path: &Path,
    is_dir: bool,
    password: Option<&str>,
) -> Result<PathBuf, String> {
    let entry_path = entry_path.trim().trim_end_matches('/');
    if entry_path.is_empty() {
        return Err(if is_dir {
            "请选择要拖出的文件夹。".to_string()
        } else {
            "请选择要拖出的文件。".to_string()
        });
    }
    if archive_entry_has_unsafe_path(entry_path) {
        return Err("该条目路径不安全，已阻止拖出。".to_string());
    }

    let output_parent = promised_path
        .parent()
        .ok_or_else(|| "无法确定拖拽目标位置。".to_string())?;
    fs::create_dir_all(output_parent).map_err(|err| format!("无法创建目标目录：{err}"))?;

    if is_single_stream_format(archive_path) {
        if is_dir {
            return Err("单文件压缩流不能作为文件夹拖出。".to_string());
        }
        extract_single_stream_to_file(archive_path, promised_path)?;
        return Ok(promised_path.to_path_buf());
    }

    let temp_dir = promise_extract_temp_dir(archive_path);
    fs::create_dir_all(&temp_dir).map_err(|err| format!("无法创建临时解压目录：{err}"))?;

    let result = (|| {
        let entries = if is_dir {
            archive_entries_for_promised_folder(archive_path, entry_path, password)?
        } else {
            vec![entry_path.to_string()]
        };
        let extracted_paths = extract_entries_to_dir(archive_path, &temp_dir, &entries, password)?;
        ensure_copyable_temp_tree_permissions(&temp_dir)?;
        let extracted_path = temp_dir.join(strip_archive_entry_prefix(entry_path));

        if is_dir {
            if extracted_path.is_dir() {
                copy_path_recursive(&extracted_path, promised_path)?;
                return Ok(promised_path.to_path_buf());
            }

            copy_promised_folder_entries_from_temp(&temp_dir, entry_path, &entries, promised_path)?;
            return Ok(promised_path.to_path_buf());
        }

        let extracted_file_path = extracted_paths.first().cloned().unwrap_or(extracted_path);

        if !extracted_file_path.is_file() {
            return Err("拖出的条目不是可写入的文件。".to_string());
        }

        fs::copy(&extracted_file_path, promised_path)
            .map_err(|err| format!("无法写入拖拽目标文件：{err}"))?;
        Ok(promised_path.to_path_buf())
    })();

    let _ = fs::remove_dir_all(&temp_dir);
    result
}

fn archive_entries_for_promised_folder(
    archive_path: &Path,
    folder_path: &str,
    password: Option<&str>,
) -> Result<Vec<String>, String> {
    let folder_path = folder_path.trim().trim_end_matches('/');
    let folder_prefix = format!("{folder_path}/");
    let entries: Vec<ExtractTaskEntry> = list_archive_entries_for_task(archive_path, password)?
        .into_iter()
        .filter(|entry| {
            let clean_path = entry.path.trim_end_matches('/');
            clean_path == folder_path || clean_path.starts_with(&folder_prefix)
        })
        .collect();

    if entries.is_empty() {
        return Err("找不到要拖出的文件夹。".to_string());
    }
    ensure_safe_archive_entry_paths(&entries)?;

    Ok(entries.into_iter().map(|entry| entry.path).collect())
}

fn copy_promised_folder_entries_from_temp(
    temp_dir: &Path,
    folder_path: &str,
    entries: &[String],
    promised_path: &Path,
) -> Result<(), String> {
    fs::create_dir_all(promised_path).map_err(|err| format!("无法创建拖拽目标文件夹：{err}"))?;

    let folder_path = folder_path.trim().trim_end_matches('/');
    let folder_prefix = format!("{folder_path}/");
    for entry in entries {
        let clean_entry = entry.trim_end_matches('/');
        if clean_entry == folder_path {
            continue;
        }
        let Some(relative_entry) = clean_entry.strip_prefix(&folder_prefix) else {
            continue;
        };
        let source = temp_dir.join(strip_archive_entry_prefix(clean_entry));
        if !source.exists() {
            continue;
        }
        let target = promised_path.join(strip_archive_entry_prefix(relative_entry));
        copy_path_recursive(&source, &target)?;
    }

    Ok(())
}

fn extract_single_stream_to_file(archive_path: &Path, output_path: &Path) -> Result<(), String> {
    let (program, args) = single_stream_decompress_command(archive_path)?;

    let output = run_command_bytes(program, args.iter().map(|arg| arg.as_os_str()), None)?;

    let mut file =
        File::create(output_path).map_err(|err| format!("无法创建拖拽目标文件：{err}"))?;
    file.write_all(&output)
        .map_err(|err| format!("无法写入拖拽目标文件：{err}"))?;
    Ok(())
}

fn promise_extract_temp_dir(archive_path: &Path) -> PathBuf {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let index = EXTRACT_TASK_COUNTER.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir()
        .join("cardiganzip-promise")
        .join(format!(
            "{}-{seed}-{index}",
            sanitize_path_segment(&file_name(archive_path))
        ))
}

fn promised_archive_entry_name(entry_path: &str, suggested_name: &str) -> String {
    let trimmed = suggested_name.trim();
    if !trimmed.is_empty() {
        let sanitized = sanitize_path_segment(trimmed);
        if sanitized != "." && sanitized != ".." {
            return sanitized;
        }
    }

    strip_archive_entry_prefix(entry_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_path_segment)
        .filter(|name| name != "." && name != "..")
        .unwrap_or_else(|| "archive-entry".to_string())
}

fn preview_output_dir(archive_path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::metadata(archive_path).map_err(|err| err.to_string())?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let name = sanitize_path_segment(&file_name(archive_path));

    Ok(std::env::temp_dir()
        .join("cardiganzip-preview")
        .join(format!("{name}-{modified}")))
}

fn strip_archive_entry_prefix(entry: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for component in Path::new(entry.trim_end_matches('/')).components() {
        if let std::path::Component::Normal(value) = component {
            path.push(value);
        }
    }
    path
}

fn sanitize_path_segment(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => ch,
        })
        .collect();

    if cleaned.trim().is_empty() {
        "archive".to_string()
    } else {
        cleaned
    }
}

fn collect_folders(entries: &[ArchiveEntry]) -> Vec<String> {
    let mut folders = BTreeSet::new();
    for entry in entries {
        let mut current = PathBuf::new();
        let path = Path::new(&entry.path);
        let components: Vec<_> = path.components().collect();
        let folder_components = if entry.kind == "folder" {
            components.len()
        } else {
            components.len().saturating_sub(1)
        };

        for component in components.into_iter().take(folder_components) {
            current.push(component.as_os_str());
            if !current.as_os_str().is_empty() {
                folders.insert(current.to_string_lossy().to_string());
            }
        }
    }

    folders.into_iter().collect()
}

fn archive_safety_summary(entries: &[ArchiveEntry]) -> ArchiveSafetySummary {
    let mut samples = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.is_unsafe_path || entry.is_hidden || entry.is_executable)
    {
        if samples.len() >= 4 {
            break;
        }
        samples.push(entry.path.clone());
    }

    ArchiveSafetySummary {
        unsafe_paths: entries.iter().filter(|entry| entry.is_unsafe_path).count(),
        hidden_files: entries.iter().filter(|entry| entry.is_hidden).count(),
        executables: entries.iter().filter(|entry| entry.is_executable).count(),
        samples,
    }
}

fn archive_entry_is_hidden(path: &str) -> bool {
    path.replace('\\', "/")
        .split('/')
        .any(|part| part.starts_with('.') && part.len() > 1)
}

fn archive_entry_is_executable(path: &str, permissions: &str) -> bool {
    if permissions
        .chars()
        .skip(1)
        .take(9)
        .any(|ch| ch == 'x' || ch == 's')
    {
        return true;
    }

    let lower = path.to_ascii_lowercase();
    [
        ".app", ".command", ".exe", ".bat", ".cmd", ".com", ".msi", ".pkg", ".dmg", ".sh", ".bash",
        ".zsh", ".fish", ".ps1", ".py", ".rb", ".pl", ".jar",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

fn archive_entry_has_unsafe_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains('\0') {
        return true;
    }
    normalized
        .split('/')
        .any(|part| part == ".." || part == "." || part.contains(':'))
}

fn ensure_safe_archive_entry_paths(entries: &[ExtractTaskEntry]) -> Result<(), String> {
    if let Some(entry) = entries.iter().find(|entry| entry.is_unsafe_path) {
        Err(format!(
            "压缩包包含不安全路径，已阻止解压：{}",
            entry.display_path
        ))
    } else {
        Ok(())
    }
}

fn run_command<'a, I>(program: &str, args: I, current_dir: Option<&Path>) -> Result<String, String>
where
    I: Iterator<Item = &'a OsStr>,
{
    let output = run_command_bytes(program, args, current_dir)?;
    Ok(String::from_utf8_lossy(&output).to_string())
}

fn run_command_with_env<'a, I>(
    program: &str,
    args: I,
    current_dir: Option<&Path>,
    disable_macos_metadata: bool,
) -> Result<String, String>
where
    I: Iterator<Item = &'a OsStr>,
{
    let output = run_command_bytes_with_env(program, args, current_dir, disable_macos_metadata)?;
    Ok(String::from_utf8_lossy(&output).to_string())
}

fn run_command_bytes<'a, I>(
    program: &str,
    args: I,
    current_dir: Option<&Path>,
) -> Result<Vec<u8>, String>
where
    I: Iterator<Item = &'a OsStr>,
{
    run_command_bytes_with_env(program, args, current_dir, false)
}

fn run_command_bytes_with_env<'a, I>(
    program: &str,
    args: I,
    current_dir: Option<&Path>,
    disable_macos_metadata: bool,
) -> Result<Vec<u8>, String>
where
    I: Iterator<Item = &'a OsStr>,
{
    let args: Vec<OsString> = args.map(OsString::from).collect();
    let candidates = command_candidates(program);

    for (index, candidate) in candidates.iter().enumerate() {
        let mut command = Command::new(candidate);
        command.args(&args);
        if disable_macos_metadata {
            command.env("COPYFILE_DISABLE", "1");
            command.env("COPY_EXTENDED_ATTRIBUTES_DISABLE", "1");
        }
        if let Some(current_dir) = current_dir {
            command.current_dir(current_dir);
        }

        match command.output() {
            Ok(output) if output.status.success() => return Ok(output.stdout),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if stderr.is_empty() {
                    format!("{program} 执行失败。")
                } else {
                    stderr
                });
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && index + 1 < candidates.len() => {
            }
            Err(_) => {
                return Err(format!(
                    "找不到命令：{program}。请确认系统已安装对应压缩工具。"
                ));
            }
        }
    }

    Err(format!(
        "找不到命令：{program}。请确认系统已安装对应压缩工具。"
    ))
}

fn command_candidates(program: &str) -> Vec<OsString> {
    if program.contains('/') {
        return vec![OsString::from(program)];
    }

    [
        program.to_string(),
        format!("/usr/bin/{program}"),
        format!("/bin/{program}"),
        format!("/opt/homebrew/bin/{program}"),
        format!("/usr/local/bin/{program}"),
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}

fn ensure_file(path: &Path) -> Result<(), String> {
    if path.is_file() {
        Ok(())
    } else {
        Err("请选择有效的压缩包文件。".to_string())
    }
}

fn ensure_supported_extract_format(path: &Path) -> Result<(), String> {
    if let Some(message) = split_archive_open_error(path) {
        return Err(message);
    }
    let format = archive_format(path);
    if SUPPORTED_EXTRACT_FORMATS.contains(&format.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "当前仅支持解压 {SUPPORTED_EXTRACT_FORMATS_LABEL}。"
        ))
    }
}

fn is_zip_container_file(path: &Path) -> bool {
    let mut signature = [0_u8; 4];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    if file.read_exact(&mut signature).is_err() {
        return false;
    }

    matches!(
        signature,
        [0x50, 0x4b, 0x03, 0x04] | [0x50, 0x4b, 0x05, 0x06] | [0x50, 0x4b, 0x07, 0x08]
    )
}

fn normalize_compress_format(format: &str) -> Result<String, String> {
    let normalized = format.trim().to_uppercase();
    match normalized.as_str() {
        "GZIP" | "TAR.GZ" | "TARGZ" => Ok("GZ".to_string()),
        "BZIP2" | "TAR.BZ2" | "TARBZ2" => Ok("BZ2".to_string()),
        "TBZ2" | "TAR.BZIP2" | "TARBZIP2" => Ok("TBZ".to_string()),
        "TAR.XZ" | "TARXZ" => Ok("TXZ".to_string()),
        "ZST" | "TAR.ZST" | "TAR.ZSTD" | "TARZST" | "TARZSTD" => Ok("ZSTD".to_string()),
        "TAR.LZMA" | "TARLZMA" => Ok("LZMA".to_string()),
        "TAR.LZMA2" | "TARLZMA2" => Ok("LZMA2".to_string()),
        "TAR.LZ4" | "TARLZ4" => Ok("LZ4".to_string()),
        "ZIP" | "7Z" | "TAR" | "TGZ" | "TBZ" | "TXZ" | "GZ" | "BZ2" | "XZ" | "Z" | "ZSTD"
        | "LZMA" | "LZMA2" | "LZ4" => Ok(normalized),
        _ => Err(format!(
            "当前仅支持压缩为 {SUPPORTED_COMPRESS_FORMATS_LABEL}。"
        )),
    }
}

fn sanitize_archive_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("请输入压缩包名称。".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed == "." || trimmed == ".." {
        return Err("压缩包名称不能包含路径分隔符。".to_string());
    }
    Ok(trimmed.to_string())
}

fn default_archive_name_for_source(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名");
    let lower = name.to_ascii_lowercase();
    let trimmed = if lower.ends_with(".tar.gz") {
        &name[..name.len().saturating_sub(".tar.gz".len())]
    } else if lower.ends_with(".tar.bz2") {
        &name[..name.len().saturating_sub(".tar.bz2".len())]
    } else if lower.ends_with(".tar.xz") {
        &name[..name.len().saturating_sub(".tar.xz".len())]
    } else {
        Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(name)
    };
    sanitize_path_segment(trimmed)
}

fn unique_output_path(output_dir: &Path, archive_name: &str, format: &str) -> PathBuf {
    let extension = compress_output_extension(format);
    let mut candidate = output_dir.join(format!("{archive_name}.{extension}"));
    let mut index = 2;
    while candidate.exists() {
        candidate = output_dir.join(format!("{archive_name} {index}.{extension}"));
        index += 1;
    }
    candidate
}

fn unique_split_7z_output_path(output_dir: &Path, archive_name: &str) -> PathBuf {
    let mut candidate = output_dir.join(format!("{archive_name}.7z.001"));
    let mut index = 2;
    while candidate.exists() || split_7z_sibling_exists(&candidate) {
        candidate = output_dir.join(format!("{archive_name} {index}.7z.001"));
        index += 1;
    }
    candidate
}

fn split_7z_sibling_exists(first_volume: &Path) -> bool {
    let Some(stem) = first_volume
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".001"))
    else {
        return false;
    };
    let parent = first_volume.parent().unwrap_or_else(|| Path::new(""));
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(stem)
                && name.rsplit_once('.').is_some_and(|(_, ext)| {
                    ext.len() == 3 && ext.chars().all(|ch| ch.is_ascii_digit())
                })
            {
                return true;
            }
        }
    }
    false
}

fn unique_temp_output_path(output_dir: &Path, archive_name: &str, format: &str) -> PathBuf {
    let extension = compress_output_extension(format);
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let mut candidate = output_dir.join(format!(".{archive_name}.packo-{seed}.{extension}"));
    let mut index = 2;
    while candidate.exists() {
        candidate = output_dir.join(format!(".{archive_name}.packo-{seed}-{index}.{extension}"));
        index += 1;
    }
    candidate
}

fn seven_zip_settings(
    options: &CompressOptions,
    password: &str,
) -> Result<SevenZipCompressSettings, String> {
    let compression_level = options.compression_level.unwrap_or(6);
    if compression_level > 9 {
        return Err("7Z 压缩级别必须在 0 到 9 之间。".to_string());
    }
    let method = match options
        .advanced
        .method
        .as_deref()
        .unwrap_or("LZMA2")
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "" | "LZMA2" => SevenZipMethod::Lzma2,
        "LZMA" => SevenZipMethod::Lzma,
        _ => return Err("7Z 当前支持 LZMA2 和 LZMA 压缩方法。".to_string()),
    };
    let threads = options.advanced.threads.unwrap_or(1).clamp(1, 32);
    if let Some(dictionary_size_mb) = options.advanced.dictionary_size_mb {
        if !(1..=1024).contains(&dictionary_size_mb) {
            return Err("7Z 字典大小必须在 1 到 1024 MB 之间。".to_string());
        }
    }

    Ok(SevenZipCompressSettings {
        password: (!password.is_empty()).then(|| password.to_string()),
        compression_level,
        dictionary_size_mb: options.advanced.dictionary_size_mb,
        threads,
        solid: options.advanced.solid.unwrap_or(false),
        method,
    })
}

fn compress_output_extension(format: &str) -> &'static str {
    match format {
        "ZIP" => "zip",
        "7Z" => "7z",
        "TAR" => "tar",
        "TGZ" => "tgz",
        "TBZ" => "tbz",
        "TXZ" => "txz",
        "GZ" => "tar.gz",
        "BZ2" => "tar.bz2",
        "XZ" => "tar.xz",
        "Z" => "tar.Z",
        "ZSTD" => "tar.zst",
        "LZMA" => "tar.lzma",
        "LZMA2" => "tar.lzma2",
        "LZ4" => "tar.lz4",
        _ => "tar",
    }
}

fn common_parent(paths: &[String]) -> Result<PathBuf, String> {
    let first = PathBuf::from(paths.first().ok_or("请先选择文件。")?);
    let mut parent = first
        .parent()
        .ok_or("无法确定文件所在目录。")?
        .to_path_buf();

    for path in paths.iter().skip(1) {
        let path = PathBuf::from(path);
        let current_parent = path.parent().ok_or("无法确定文件所在目录。")?;
        while !current_parent.starts_with(&parent) {
            if !parent.pop() {
                return Err("无法计算公共目录。".to_string());
            }
        }
    }

    Ok(parent)
}

fn archive_format(path: &Path) -> String {
    if let Some(format) = split_archive_format(path) {
        return format.to_string();
    }

    let name = file_name(path).to_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tar.gzip") || name.ends_with(".tgz") {
        "TGZ".to_string()
    } else if name.ends_with(".tar.bz2")
        || name.ends_with(".tar.bzip2")
        || name.ends_with(".tbz")
        || name.ends_with(".tbz2")
    {
        "TBZ".to_string()
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        "TXZ".to_string()
    } else if name.ends_with(".tar.zst") || name.ends_with(".tar.zstd") || name.ends_with(".tzst") {
        "ZSTD".to_string()
    } else if name.ends_with(".tar.lzma2") {
        "LZMA2".to_string()
    } else if name.ends_with(".tar.lzma") || name.ends_with(".tlzma") {
        "LZMA".to_string()
    } else if name.ends_with(".tar.lz4") || name.ends_with(".tlz4") {
        "LZ4".to_string()
    } else if name.ends_with(".tar.z") {
        "Z".to_string()
    } else {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "gz" => "GZ".to_string(),
            "gzip" => "GZIP".to_string(),
            "bz2" => "BZ2".to_string(),
            "bzip2" => "BZIP2".to_string(),
            "xz" => "XZ".to_string(),
            "z" => "Z".to_string(),
            "zst" | "zstd" => "ZSTD".to_string(),
            "lzma" => "LZMA".to_string(),
            "lzma2" => "LZMA2".to_string(),
            "lz4" => "LZ4".to_string(),
            "tgz" => "TGZ".to_string(),
            "tbz" | "tbz2" => "TBZ".to_string(),
            "txz" => "TXZ".to_string(),
            "lha" => "LHA".to_string(),
            "lzh" => "LZH".to_string(),
            _ if is_zip_container_file(path) => "ZIP".to_string(),
            ext => ext.to_uppercase(),
        }
    }
}

fn split_archive_format(path: &Path) -> Option<&'static str> {
    let name = file_name(path).to_ascii_lowercase();
    if rar_part_number(&name).is_some() {
        return Some("RAR");
    }
    if zip_z_volume_number(&name).is_some() {
        return Some("ZIP");
    }
    let (base, _) = numeric_split_volume_number(&name)?;
    Some(split_base_archive_format(base))
}

fn split_archive_open_error(path: &Path) -> Option<String> {
    let name = file_name(path).to_ascii_lowercase();
    if zip_z_volume_number(&name).is_some() {
        let main_name = zip_split_main_name(&name);
        return Some(format!(
            "这是 ZIP 分卷片段，请选择同名主文件 {main_name} 打开。"
        ));
    }

    if let Some(part) = rar_part_number(&name) {
        if part > 1 {
            return Some("这是 RAR 分卷片段，请选择 .part1.rar 第一卷打开。".to_string());
        }
    }

    if let Some((_, volume)) = numeric_split_volume_number(&name) {
        if volume > 1 {
            return Some("这是分卷片段，请选择 .001 第一卷打开。".to_string());
        }
    }

    None
}

fn split_base_archive_format(base: &str) -> &'static str {
    if base.ends_with(".tar.gz") || base.ends_with(".tar.gzip") || base.ends_with(".tgz") {
        "TGZ"
    } else if base.ends_with(".tar.bz2")
        || base.ends_with(".tar.bzip2")
        || base.ends_with(".tbz")
        || base.ends_with(".tbz2")
    {
        "TBZ"
    } else if base.ends_with(".tar.xz") || base.ends_with(".txz") {
        "TXZ"
    } else if base.ends_with(".tar.zst") || base.ends_with(".tar.zstd") || base.ends_with(".tzst") {
        "ZSTD"
    } else if base.ends_with(".tar.lzma2") {
        "LZMA2"
    } else if base.ends_with(".tar.lzma") || base.ends_with(".tlzma") {
        "LZMA"
    } else if base.ends_with(".tar.lz4") || base.ends_with(".tlz4") {
        "LZ4"
    } else if base.ends_with(".tar.z") {
        "Z"
    } else if base.ends_with(".zip") {
        "ZIP"
    } else if base.ends_with(".rar") {
        "RAR"
    } else if base.ends_with(".7z") {
        "7Z"
    } else if base.ends_with(".tar") {
        "TAR"
    } else {
        "7Z"
    }
}

fn rar_part_number(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".rar")?;
    let (_, part) = stem.rsplit_once(".part")?;
    if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    part.parse::<u32>().ok()
}

fn zip_z_volume_number(name: &str) -> Option<u32> {
    let (_, extension) = name.rsplit_once('.')?;
    let digits = extension.strip_prefix('z')?;
    if digits.len() != 2 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn numeric_split_volume_number(name: &str) -> Option<(&str, u32)> {
    let (base, extension) = name.rsplit_once('.')?;
    if extension.len() != 3 || !extension.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((base, extension.parse::<u32>().ok()?))
}

fn zip_split_main_name(name: &str) -> String {
    let base = name.rsplit_once('.').map(|(base, _)| base).unwrap_or(name);
    format!("{base}.zip")
}

fn zip_split_main_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let desired_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(zip_split_main_name)
        .unwrap_or_else(|| "archive.zip".to_string());
    let exact = parent.join(&desired_name);
    if exact.exists() {
        return exact;
    }

    let desired_lower = desired_name.to_ascii_lowercase();
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_ascii_lowercase() == desired_lower)
                .unwrap_or(false)
            {
                return candidate;
            }
        }
    }

    exact
}

fn is_single_stream_format(path: &Path) -> bool {
    if has_compressed_tar_extension(path) {
        return false;
    }
    matches!(
        archive_format(path).as_str(),
        "GZ" | "GZIP" | "BZ2" | "BZIP2" | "XZ" | "Z" | "ZSTD" | "LZMA" | "LZMA2" | "LZ4"
    )
}

fn has_compressed_tar_extension(path: &Path) -> bool {
    let name = file_name(path).to_lowercase();
    name.ends_with(".tar.gz")
        || name.ends_with(".tar.gzip")
        || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2")
        || name.ends_with(".tar.bzip2")
        || name.ends_with(".tbz")
        || name.ends_with(".tbz2")
        || name.ends_with(".tar.xz")
        || name.ends_with(".txz")
        || name.ends_with(".tar.z")
        || name.ends_with(".tar.zst")
        || name.ends_with(".tar.zstd")
        || name.ends_with(".tzst")
        || name.ends_with(".tar.lzma")
        || name.ends_with(".tar.lzma2")
        || name.ends_with(".tlzma")
        || name.ends_with(".tar.lz4")
        || name.ends_with(".tlz4")
}

fn single_stream_decompress_command(path: &Path) -> Result<(&'static str, Vec<OsString>), String> {
    let mut args = Vec::new();
    match archive_format(path).as_str() {
        "GZ" | "GZIP" => {
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("gzip", args))
        }
        "BZ2" | "BZIP2" => {
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("bzip2", args))
        }
        "XZ" | "LZMA2" => {
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("xz", args))
        }
        "Z" => {
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("gzip", args))
        }
        "ZSTD" => {
            args.push(OsString::from("-q"));
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("zstd", args))
        }
        "LZMA" => {
            args.push(OsString::from("--format=lzma"));
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("xz", args))
        }
        "LZ4" => {
            args.push(OsString::from("-q"));
            args.push(OsString::from("-dc"));
            args.push(path.as_os_str().to_os_string());
            Ok(("lz4", args))
        }
        _ => Err("当前文件不是单文件压缩流。".to_string()),
    }
}

fn single_stream_output_name(path: &Path) -> String {
    let name = file_name(path);
    let lower = name.to_lowercase();
    for extension in [
        ".bzip2", ".gzip", ".lzma2", ".zstd", ".lzma", ".bz2", ".gz", ".xz", ".zst", ".lz4", ".z",
    ] {
        if lower.ends_with(extension) {
            return name[..name.len().saturating_sub(extension.len())].to_string();
        }
    }
    format!("{name}.out")
}

fn unique_file_path(output_dir: &Path, file_name: &str) -> PathBuf {
    let original = Path::new(file_name);
    let stem = original
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("未命名");
    let extension = original.extension().and_then(|ext| ext.to_str());
    let mut candidate = output_dir.join(file_name);
    let mut index = 2;

    while candidate.exists() {
        let next_name = match extension {
            Some(extension) if !extension.is_empty() => format!("{stem} {index}.{extension}"),
            _ => format!("{stem} {index}"),
        };
        candidate = output_dir.join(next_name);
        index += 1;
    }

    candidate
}

fn file_kind(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "zip" | "rar" | "7z" | "tar" | "gz" | "gzip" | "bz2" | "bzip2" | "xz" | "tgz" | "tbz"
        | "tbz2" | "txz" | "lzh" | "lha" | "z" | "zst" | "zstd" | "tzst" | "lzma" | "lzma2"
        | "tlzma" | "lz4" | "tlz4" | "iso" => "archive",
        "pdf" => "pdf",
        "doc" | "docx" => "word",
        "xls" | "xlsx" => "excel",
        "png" | "jpg" | "jpeg" | "gif" | "webp" => "image",
        "txt" | "md" | "json" | "csv" => "text",
        _ => "file",
    }
    .to_string()
}

fn extension_label(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("文件")
        .to_uppercase()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string()
}

fn format_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", size, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn work_window_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn work_window_counter_suffix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let counter = WORK_WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{millis:x}-{counter:x}")
}

fn work_window_url() -> tauri::WebviewUrl {
    tauri::WebviewUrl::App(PathBuf::from("index.html"))
}

fn normalize_existing_system_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for path in paths {
        if path.as_os_str().is_empty() || !path.exists() {
            continue;
        }
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            normalized.push(path);
        }
    }
    normalized
}

fn is_supported_archive_path(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    normalize_archive_open_path(path)
        .and_then(|path| {
            ensure_file(&path)?;
            ensure_supported_extract_format(&path)
        })
        .is_ok()
}

fn route_system_open_paths<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let paths = normalize_existing_system_paths(paths);
    if paths.is_empty() {
        focus_main_window(app);
        return Ok(());
    }

    hide_main_window(app);
    let payloads = app.state::<WorkWindowPayloads>();
    let all_archives = paths.iter().all(|path| is_supported_archive_path(path));
    let result = if all_archives {
        paths
            .into_iter()
            .try_for_each(|path| open_extract_work_window_for_path(app, &payloads, &path, true))
    } else {
        open_compress_work_window_for_paths(app, &payloads, paths, false)
    };

    if result.is_err() {
        focus_main_window(app);
    }
    result
}

fn route_opened_urls<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    urls: Vec<url::Url>,
) -> Result<(), String> {
    let paths = urls
        .into_iter()
        .filter_map(|url| url.to_file_path().ok())
        .collect();
    route_system_open_paths(app, paths)
}

fn focus_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn focus_window_when_still_hidden<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
    label: String,
    delay: Duration,
) {
    thread::spawn(move || {
        thread::sleep(delay);
        if let Some(window) = app.get_webview_window(&label) {
            if !window.is_visible().unwrap_or(false) {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });
}

fn hide_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn emit_menu_action<R: tauri::Runtime>(app: &tauri::AppHandle<R>, action: &str) {
    let _ = app.emit("packo-menu-action", action);
}

fn open_extract_work_window_for_path<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    payloads: &tauri::State<'_, WorkWindowPayloads>,
    path: &Path,
    focus_existing: bool,
) -> Result<(), String> {
    let open_path = normalize_archive_open_path(path).unwrap_or_else(|_| path.to_path_buf());
    let path_string = open_path.to_string_lossy().to_string();
    let title = format!(
        "{} - Packo",
        open_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("压缩包")
    );
    let label = format!("packo-extract-{}", work_window_hash(&path_string));

    open_packo_work_window(
        app,
        payloads,
        label,
        title,
        WorkWindowPayload::Extract {
            paths: vec![path_string],
        },
        focus_existing,
    )
}

fn open_compress_work_window_for_paths<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    payloads: &tauri::State<'_, WorkWindowPayloads>,
    paths: Vec<PathBuf>,
    focus_existing: bool,
) -> Result<(), String> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .filter(|path| !path.trim().is_empty())
        .collect();

    let title = if paths.is_empty() {
        "新建压缩包 - Packo".to_string()
    } else if paths.len() == 1 {
        let name = Path::new(&paths[0])
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("新建压缩包");
        format!("压缩 {name} - Packo")
    } else {
        format!("压缩 {} 个项目 - Packo", paths.len())
    };
    let label = format!("packo-compress-{}", work_window_counter_suffix());

    open_packo_work_window(
        app,
        payloads,
        label,
        title,
        WorkWindowPayload::Compress { paths },
        focus_existing,
    )
}

fn extract_archives_from_finder_service(
    paths: Vec<PathBuf>,
    destination: FinderExtractDestination,
) {
    let paths = normalize_existing_system_paths(paths)
        .into_iter()
        .filter(|path| is_supported_archive_path(path))
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return;
    }

    thread::spawn(move || {
        for path in paths {
            let result = extract_archive_from_finder_service(&path, destination);
            if let Err(error) = result {
                eprintln!(
                    "Packo Finder extract service failed for {}: {error}",
                    path.to_string_lossy()
                );
            }
        }
    });
}

fn extract_archive_from_finder_service(
    archive_path: &Path,
    destination: FinderExtractDestination,
) -> Result<(), String> {
    let archive_path = normalize_archive_open_path(archive_path)?;
    ensure_file(&archive_path)?;
    ensure_supported_extract_format(&archive_path)?;

    let parent = archive_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let output_dir = match destination {
        FinderExtractDestination::CurrentDirectory => parent,
        FinderExtractDestination::ArchiveNamedFolder => {
            let mut reserved = BTreeSet::new();
            unique_directory_path(
                &parent,
                &archive_output_folder_name(&archive_path),
                &mut reserved,
            )
        }
    };

    extract_archive_items(
        archive_path.to_string_lossy().to_string(),
        output_dir.to_string_lossy().to_string(),
        Vec::new(),
        None,
    )?;
    Ok(())
}

fn open_packo_work_window<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    payloads: &tauri::State<'_, WorkWindowPayloads>,
    label: String,
    title: String,
    payload: WorkWindowPayload,
    focus_existing: bool,
) -> Result<(), String> {
    use tauri::Manager;

    if focus_existing {
        if let Some(window) = app.get_webview_window(&label) {
            window
                .show()
                .map_err(|err| format!("无法显示窗口：{err}"))?;
            window
                .set_focus()
                .map_err(|err| format!("无法聚焦窗口：{err}"))?;
            return Ok(());
        }
    }

    {
        let mut payload_map = payloads
            .payloads
            .lock()
            .map_err(|_| "无法创建工作窗口。".to_string())?;
        payload_map.insert(label.clone(), payload);
    }

    let mut builder = tauri::WebviewWindowBuilder::new(app, &label, work_window_url())
        .title(title)
        .inner_size(920.0, 620.0)
        .min_inner_size(760.0, 520.0)
        .decorations(true)
        .resizable(true)
        .center()
        .focused(true)
        .background_color(tauri::utils::config::Color(255, 253, 249, 255))
        .visible(false);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(20.0, 35.0));
    }

    match builder.build() {
        Ok(_) => {
            focus_window_when_still_hidden(app.clone(), label, Duration::from_millis(900));
            Ok(())
        }
        Err(error) => {
            if let Ok(mut payload_map) = payloads.payloads.lock() {
                payload_map.remove(&label);
            }
            Err(format!("无法打开工作窗口：{error}"))
        }
    }
}

#[tauri::command]
async fn open_extract_window<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
    payloads: tauri::State<'_, WorkWindowPayloads>,
    path: String,
) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("请先选择压缩包。".to_string());
    }

    let requested_path = PathBuf::from(trimmed);
    open_extract_work_window_for_path(&app, &payloads, &requested_path, true)
}

#[tauri::command]
async fn open_compress_window<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
    payloads: tauri::State<'_, WorkWindowPayloads>,
    paths: Vec<String>,
) -> Result<(), String> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect();

    open_compress_work_window_for_paths(
        &app,
        &payloads,
        paths.into_iter().map(PathBuf::from).collect(),
        false,
    )
}

#[tauri::command]
fn get_work_window_payload<R: tauri::Runtime>(
    window: tauri::Window<R>,
    payloads: tauri::State<'_, WorkWindowPayloads>,
) -> Result<Option<WorkWindowPayload>, String> {
    let label = window.label().to_string();
    let payload = payloads
        .payloads
        .lock()
        .map_err(|_| "无法读取工作窗口数据。".to_string())?
        .get(&label)
        .cloned();
    Ok(payload)
}

fn format_system_time(time: std::time::SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    format!(
        "{}年{}月{}日 {:02}:{:02}",
        datetime.year(),
        datetime.month(),
        datetime.day(),
        datetime.hour(),
        datetime.minute()
    )
}

#[cfg(target_os = "macos")]
fn run_pick_compress_sources_panel() -> Result<Vec<String>, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::NSString;

    let mtm =
        MainThreadMarker::new().ok_or_else(|| "文件选择面板必须在主线程打开。".to_string())?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(true);
    panel.setResolvesAliases(true);
    panel.setCanCreateDirectories(false);
    panel.setTitle(Some(&NSString::from_str("选择要压缩的项目")));
    panel.setMessage(Some(&NSString::from_str("可同时选择文件和文件夹。")));
    panel.setPrompt(Some(&NSString::from_str("选择")));

    if panel.runModal() != NSModalResponseOK {
        return Ok(Vec::new());
    }

    let urls = panel.URLs();
    let mut paths = Vec::with_capacity(urls.count());
    for index in 0..urls.count() {
        if let Some(path) = urls.objectAtIndex(index).path() {
            paths.push(path.to_string());
        }
    }
    Ok(paths)
}

#[tauri::command]
fn pick_compress_sources<R: tauri::Runtime>(
    window: tauri::Window<R>,
) -> Result<Vec<String>, String> {
    #[cfg(target_os = "macos")]
    {
        if objc2::MainThreadMarker::new().is_some() {
            return run_pick_compress_sources_panel();
        }

        let (sender, receiver) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(run_pick_compress_sources_panel());
            })
            .map_err(|err| format!("无法打开文件选择面板：{err}"))?;

        receiver
            .recv()
            .map_err(|err| format!("文件选择面板没有返回结果：{err}"))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("当前平台暂不支持同时选择文件和文件夹。".to_string())
    }
}

#[cfg(target_os = "macos")]
fn run_pick_open_with_application_panel() -> Result<Option<OpenWithApplication>, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{NSString, NSURL};

    let mtm =
        MainThreadMarker::new().ok_or_else(|| "应用选择面板必须在主线程打开。".to_string())?;
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    panel.setResolvesAliases(true);
    panel.setCanCreateDirectories(false);
    panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(
        "/Applications",
    ))));
    panel.setTitle(Some(&NSString::from_str("选择打开方式")));
    panel.setMessage(Some(&NSString::from_str("请选择一个应用程序打开该文件。")));
    panel.setPrompt(Some(&NSString::from_str("打开")));

    if panel.runModal() != NSModalResponseOK {
        return Ok(None);
    }

    let Some(url) = panel.URL() else {
        return Ok(None);
    };
    let Some(path) = url.path() else {
        return Ok(None);
    };
    let path = path.to_string();
    let name = Path::new(&path)
        .file_stem()
        .or_else(|| Path::new(&path).file_name())
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("应用程序")
        .to_string();

    Ok(Some(OpenWithApplication {
        name,
        path,
        is_default: false,
    }))
}

#[tauri::command]
fn pick_open_with_application<R: tauri::Runtime>(
    window: tauri::Window<R>,
) -> Result<Option<OpenWithApplication>, String> {
    #[cfg(target_os = "macos")]
    {
        if objc2::MainThreadMarker::new().is_some() {
            return run_pick_open_with_application_panel();
        }

        let (sender, receiver) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(run_pick_open_with_application_panel());
            })
            .map_err(|err| format!("无法打开应用选择面板：{err}"))?;

        receiver
            .recv()
            .map_err(|err| format!("应用选择面板没有返回结果：{err}"))?
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        Err("当前平台暂不支持选择打开方式。".to_string())
    }
}

fn build_app_menu<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let about_metadata = AboutMetadata {
        name: Some("Packo".to_string()),
        version: Some(app_handle.package_info().version.to_string()),
        icon: Some(tauri::image::Image::new(
            include_bytes!("../icons/about-logo.rgba"),
            720,
            260,
        )),
        ..Default::default()
    };

    let new_archive_item = MenuItem::with_id(
        app_handle,
        "packo-new-archive",
        "新建压缩包",
        true,
        Some("CmdOrCtrl+N"),
    )?;
    let open_archive_item = MenuItem::with_id(
        app_handle,
        "packo-open-archive",
        "打开压缩包...",
        true,
        Some("CmdOrCtrl+O"),
    )?;
    let extract_archive_item = MenuItem::with_id(
        app_handle,
        "packo-extract-archive",
        "解压",
        true,
        Some("CmdOrCtrl+E"),
    )?;
    let recent_item = MenuItem::with_id(
        app_handle,
        "packo-show-recent",
        "最近文件",
        true,
        Some("CmdOrCtrl+F"),
    )?;
    let settings_item = MenuItem::with_id(
        app_handle,
        "packo-show-settings",
        "设置...",
        true,
        Some("CmdOrCtrl+,"),
    )?;

    #[cfg(target_os = "macos")]
    let app_menu = Submenu::with_items(
        app_handle,
        "Packo",
        true,
        &[
            &PredefinedMenuItem::about(
                app_handle,
                Some("关于 Packo"),
                Some(about_metadata.clone()),
            )?,
            &PredefinedMenuItem::separator(app_handle)?,
            &settings_item,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::services(app_handle, Some("服务"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::hide(app_handle, Some("隐藏 Packo"))?,
            &PredefinedMenuItem::hide_others(app_handle, Some("隐藏其他"))?,
            &PredefinedMenuItem::show_all(app_handle, Some("全部显示"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::quit(app_handle, Some("退出 Packo"))?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        app_handle,
        "文件",
        true,
        &[
            &new_archive_item,
            &open_archive_item,
            &PredefinedMenuItem::separator(app_handle)?,
            &extract_archive_item,
            &PredefinedMenuItem::separator(app_handle)?,
            &recent_item,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::close_window(app_handle, Some("关闭窗口"))?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app_handle, Some("退出 Packo"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app_handle,
        "编辑",
        true,
        &[
            &PredefinedMenuItem::undo(app_handle, Some("撤销"))?,
            &PredefinedMenuItem::redo(app_handle, Some("重做"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::cut(app_handle, Some("剪切"))?,
            &PredefinedMenuItem::copy(app_handle, Some("复制"))?,
            &PredefinedMenuItem::paste(app_handle, Some("粘贴"))?,
            &PredefinedMenuItem::select_all(app_handle, Some("全选"))?,
        ],
    )?;

    let view_menu = Submenu::with_items(
        app_handle,
        "视图",
        true,
        &[&PredefinedMenuItem::fullscreen(
            app_handle,
            Some("进入全屏"),
        )?],
    )?;

    let window_menu = Submenu::with_items(
        app_handle,
        "窗口",
        true,
        &[
            &PredefinedMenuItem::minimize(app_handle, Some("最小化"))?,
            &PredefinedMenuItem::maximize(app_handle, Some("缩放"))?,
            &PredefinedMenuItem::separator(app_handle)?,
            &PredefinedMenuItem::close_window(app_handle, Some("关闭窗口"))?,
        ],
    )?;

    let help_menu = Submenu::with_items(
        app_handle,
        "帮助",
        true,
        &[&PredefinedMenuItem::about(
            app_handle,
            Some("关于 Packo"),
            Some(about_metadata),
        )?],
    )?;

    Menu::with_items(
        app_handle,
        &[
            #[cfg(target_os = "macos")]
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ],
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .enable_macos_default_menu(false)
        .menu(build_app_menu)
        .setup(|app| {
            #[cfg(target_os = "macos")]
            macos_finder_services::register(app.handle().clone());
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "packo-new-archive" {
                emit_menu_action(app, "newArchive");
            } else if event.id() == "packo-open-archive" {
                emit_menu_action(app, "openArchive");
            } else if event.id() == "packo-extract-archive" {
                emit_menu_action(app, "extractArchive");
            } else if event.id() == "packo-show-recent" {
                emit_menu_action(app, "showRecent");
            } else if event.id() == "packo-show-settings" {
                emit_menu_action(app, "showSettings");
            }
        })
        .manage(ExtractTasks::default())
        .manage(CompressTasks::default())
        .manage(WorkWindowPayloads::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            open_extract_window,
            open_compress_window,
            get_work_window_payload,
            describe_paths,
            system_file_icons,
            open_full_disk_access_settings,
            reveal_packo_app_in_finder,
            set_packo_as_default_archive_opener,
            describe_compress_paths,
            list_archive,
            test_archive_integrity,
            clear_preview_cache,
            edit_archive,
            compress_archive,
            start_compress_task,
            get_compress_task,
            pause_compress_task,
            resume_compress_task,
            cancel_compress_task,
            extract_archive,
            extract_archive_entries,
            start_extract_task,
            start_batch_extract_task,
            get_extract_task,
            pause_extract_task,
            resume_extract_task,
            cancel_extract_task,
            preview_archive_entry,
            suggest_open_with_apps,
            open_file_with_application,
            start_preview_task,
            start_archive_entry_promise_drag,
            start_archive_entries_promise_drag,
            pick_compress_sources,
            pick_open_with_application,
            default_output_dir
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let system_opened_paths = Arc::new(AtomicBool::new(false));
    app.run(move |app, event| match event {
        tauri::RunEvent::Ready => {
            let app = app.clone();
            let system_opened_paths = Arc::clone(&system_opened_paths);
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(900));
                if !system_opened_paths.load(Ordering::SeqCst) {
                    focus_main_window(&app);
                }
            });
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Opened { urls } => {
            if !urls.is_empty() {
                system_opened_paths.store(true, Ordering::SeqCst);
            }
            if let Err(error) = route_opened_urls(app, urls) {
                eprintln!("Packo failed to open system paths: {error}");
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            if !has_visible_windows {
                focus_main_window(app);
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    fn test_compress_options(
        source_paths: Vec<String>,
        output_dir: &Path,
        archive_name: &str,
        format: &str,
    ) -> CompressOptions {
        CompressOptions {
            source_paths,
            output_dir: output_dir.to_string_lossy().to_string(),
            archive_name: archive_name.to_string(),
            format: format.to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        }
    }

    #[test]
    fn formats_archive_listing_time_for_chinese_ui() {
        let current_year = Local::now().year();
        assert_eq!(
            archive_listing_time_label("Jun", "13", "19:34"),
            format!("{current_year}年6月13日 19:34")
        );
        assert_eq!(
            archive_listing_time_label("May", "6", "2024"),
            "2024年5月6日 00:00"
        );
    }

    #[test]
    fn parses_archive_paths_from_plain_listing_output() {
        let path_output = "\
开发A2501-清风润学子，廉洁启新程/
开发A2501-清风润学子，廉洁启新程/开发A2501.mp4
开发A2501-清风润学子，廉洁启新程/开发A2501-清风润学子，廉洁启新程.pptx
";
        let detail_output = "\
drwxr-xr-x  0 501    20          0 Jun 17 11:08 开发A2501-清风润学子，廉洁启新程/
-r--r--r--  0 501    20    6556043 Jun 16 20:55 开发A2501-清风润学子，廉洁启新程/开发A2501.mp4
-rw-r--r--  0 501    20   17013269 Jun 17 09:33 开发A2501-清风润学子，廉洁启新程/开发A2501-清风润学子，廉洁启新程.pptx
";

        let entries = parse_archive_listing(path_output, detail_output);

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "开发A2501-清风润学子，廉洁启新程");
        assert_eq!(
            entries[1].path,
            "开发A2501-清风润学子，廉洁启新程/开发A2501.mp4"
        );
        assert_eq!(entries[1].size, 6_556_043);
        assert_eq!(entries[2].name, "开发A2501-清风润学子，廉洁启新程.pptx");
    }

    #[test]
    fn decodes_octal_escaped_archive_paths() {
        let path_output = "\
\\345\\274\\200\\345\\217\\221A2501-清风润学子，廉洁启新程/
\\345\\274\\200\\345\\217\\221A2501-清风润学子，廉洁启新程/\\345\\274\\200\\345\\217\\221A2501.mp4
";
        let detail_output = "\
drwxr-xr-x  0 501    20          0 Jun 17 11:08 \\345\\274\\200\\345\\217\\221A2501-清风润学子，廉洁启新程/
-r--r--r--  0 501    20    6556043 Jun 16 20:55 \\345\\274\\200\\345\\217\\221A2501-清风润学子，廉洁启新程/\\345\\274\\200\\345\\217\\221A2501.mp4
";

        let entries = parse_archive_listing(path_output, detail_output);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "开发A2501-清风润学子，廉洁启新程");
        assert_eq!(
            entries[1].path,
            "开发A2501-清风润学子，廉洁启新程/开发A2501.mp4"
        );
        assert_eq!(entries[1].name, "开发A2501.mp4");
    }

    #[test]
    fn compresses_and_extracts_supported_archive_formats() {
        for format in ["ZIP", "7Z", "TAR"] {
            let temp = tempdir().expect("temp dir");
            let source = temp.path().join("hello.txt");
            fs::write(&source, format!("hello {format}")).expect("write source");

            let result = compress_archive(CompressOptions {
                source_paths: vec![source.to_string_lossy().to_string()],
                output_dir: temp.path().to_string_lossy().to_string(),
                archive_name: format!("bundle-{format}"),
                format: format.to_string(),
                compression_level: None,
                password: None,
                volume_size_mb: None,
                excluded_paths: Vec::new(),
                skip_ds_store: false,
                advanced: CompressAdvancedOptions::default(),
            })
            .expect("compress archive");

            let info = list_archive(result.output_path.clone(), None).expect("list archive");
            assert_eq!(info.format, format);
            assert_eq!(info.file_count, 1);
            assert!(info.entries.iter().any(|entry| entry.name == "hello.txt"));

            let output_dir = temp.path().join("out");
            extract_archive(
                result.output_path,
                output_dir.to_string_lossy().to_string(),
                None,
            )
            .expect("extract archive");

            assert_eq!(
                fs::read_to_string(output_dir.join("hello.txt")).expect("read extracted"),
                format!("hello {format}")
            );
        }
    }

    #[test]
    fn lists_zip_entry_properties_and_tests_integrity() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("details.txt");
        fs::write(&source, "packo details ".repeat(512)).expect("write source");

        let result = compress_archive(test_compress_options(
            vec![source.to_string_lossy().to_string()],
            temp.path(),
            "details",
            "ZIP",
        ))
        .expect("compress archive");

        let info = list_archive(result.output_path.clone(), None).expect("list archive");
        let entry = info
            .entries
            .iter()
            .find(|entry| entry.path == "details.txt")
            .expect("details entry");
        assert!(entry.crc.as_deref().is_some_and(|crc| !crc.is_empty()));
        assert!(entry
            .method
            .as_deref()
            .is_some_and(|method| !method.is_empty()));
        assert!(!entry.is_encrypted);
        assert!(info.properties.crc_available);
        assert!(!info.properties.is_encrypted);
        assert!(info.properties.uncompressed_size >= 1024);

        let tested = test_archive_integrity(result.output_path, None).expect("test archive");
        assert!(tested.message.contains("完整性测试通过"));
    }

    #[test]
    fn clears_preview_cache_for_archive() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("preview-cache.txt");
        fs::write(&source, "preview cache").expect("write source");

        let result = compress_archive(test_compress_options(
            vec![source.to_string_lossy().to_string()],
            temp.path(),
            "preview-cache",
            "ZIP",
        ))
        .expect("compress archive");
        let archive_path = PathBuf::from(&result.output_path);
        let preview = preview_archive_entry(
            result.output_path.clone(),
            "preview-cache.txt".to_string(),
            None,
        )
        .expect("preview archive entry");
        let preview_dir = preview_output_dir(&archive_path).expect("preview dir");
        assert!(PathBuf::from(preview.output_path).exists());
        assert!(preview_dir.exists());

        let cleared = clear_preview_cache(Some(result.output_path)).expect("clear preview cache");
        assert!(cleared.message.contains("已清理"));
        assert!(!preview_dir.exists());
    }

    #[test]
    fn edits_zip_archive_entries_in_place() {
        let temp = tempdir().expect("temp dir");
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).expect("create source");
        fs::write(source_dir.join("delete.txt"), "delete me").expect("write delete");
        fs::write(source_dir.join("rename.txt"), "rename me").expect("write rename");
        fs::write(source_dir.join("replace.txt"), "replace me").expect("write replace");
        let target_dir = source_dir.join("target");
        fs::create_dir_all(&target_dir).expect("create target");
        fs::write(target_dir.join("keep.txt"), "keep target").expect("write target child");
        let emptied_dir = source_dir.join("emptied");
        fs::create_dir_all(&emptied_dir).expect("create emptied folder");
        fs::write(emptied_dir.join("only.txt"), "move me out").expect("write emptied child");

        let result = compress_archive(test_compress_options(
            vec![
                source_dir.join("delete.txt").to_string_lossy().to_string(),
                source_dir.join("rename.txt").to_string_lossy().to_string(),
                source_dir.join("replace.txt").to_string_lossy().to_string(),
                target_dir.to_string_lossy().to_string(),
                emptied_dir.to_string_lossy().to_string(),
            ],
            temp.path(),
            "editable",
            "ZIP",
        ))
        .expect("compress archive");

        let add_file = temp.path().join("added.txt");
        let add_folder = temp.path().join("added-folder");
        fs::create_dir_all(&add_folder).expect("create added folder");
        fs::write(&add_file, "added file").expect("write added file");
        fs::write(add_folder.join("child.txt"), "added folder child").expect("write added child");
        let targeted_file = temp.path().join("targeted.txt");
        fs::write(&targeted_file, "targeted file").expect("write targeted file");
        let replacement = temp.path().join("replacement.txt");
        fs::write(&replacement, "replacement body").expect("write replacement");

        edit_archive(ArchiveEditOptions {
            path: result.output_path.clone(),
            password: None,
            delete_entries: vec!["delete.txt".to_string()],
            rename_entries: vec![
                ArchiveRenameEntry {
                    from: "rename.txt".to_string(),
                    to: "renamed.txt".to_string(),
                },
                ArchiveRenameEntry {
                    from: "target/keep.txt".to_string(),
                    to: "target/kept.txt".to_string(),
                },
                ArchiveRenameEntry {
                    from: "emptied/only.txt".to_string(),
                    to: "only.txt".to_string(),
                },
            ],
            add_paths: vec![
                add_file.to_string_lossy().to_string(),
                add_folder.to_string_lossy().to_string(),
            ],
            add_entries: vec![ArchiveAddEntry {
                source_path: targeted_file.to_string_lossy().to_string(),
                target_dir: "target".to_string(),
            }],
            create_dirs: vec!["created".to_string()],
            replace_entries: vec![ArchiveReplaceEntry {
                entry_path: "replace.txt".to_string(),
                source_path: replacement.to_string_lossy().to_string(),
            }],
            output_path: None,
        })
        .expect("edit archive");

        let info = list_archive(result.output_path.clone(), None).expect("list edited archive");
        let paths = info
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!paths.contains("delete.txt"));
        assert!(paths.contains("renamed.txt"));
        assert!(paths.contains("replace.txt"));
        assert!(paths.contains("added.txt"));
        assert!(paths.contains("added-folder/child.txt"));
        assert!(paths.contains("target/kept.txt"));
        assert!(paths.contains("target/targeted.txt"));
        assert!(paths.contains("only.txt"));
        assert!(paths.contains("emptied") || paths.contains("emptied/"));
        assert!(paths.contains("created") || paths.contains("created/"));

        let output_dir = temp.path().join("edited-out");
        extract_archive(
            result.output_path,
            output_dir.to_string_lossy().to_string(),
            None,
        )
        .expect("extract edited archive");
        assert_eq!(
            fs::read_to_string(output_dir.join("renamed.txt")).expect("read renamed"),
            "rename me"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("replace.txt")).expect("read replaced"),
            "replacement body"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("added.txt")).expect("read added"),
            "added file"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("added-folder").join("child.txt"))
                .expect("read added child"),
            "added folder child"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("target").join("targeted.txt"))
                .expect("read targeted child"),
            "targeted file"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("target").join("kept.txt"))
                .expect("read renamed nested child"),
            "keep target"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("only.txt")).expect("read moved child"),
            "move me out"
        );
        assert!(output_dir.join("emptied").is_dir());
        assert!(output_dir.join("created").is_dir());
        assert!(!output_dir.join("delete.txt").exists());
    }

    #[test]
    fn saves_edited_archive_as_new_file() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("original.txt");
        fs::write(&source, "original body").expect("write source");

        let result = compress_archive(test_compress_options(
            vec![source.to_string_lossy().to_string()],
            temp.path(),
            "original",
            "ZIP",
        ))
        .expect("compress archive");
        let original_path = result.output_path.clone();
        let saved_as = temp.path().join("saved-as.zip");

        edit_archive(ArchiveEditOptions {
            path: result.output_path,
            password: None,
            delete_entries: Vec::new(),
            rename_entries: vec![ArchiveRenameEntry {
                from: "original.txt".to_string(),
                to: "saved.txt".to_string(),
            }],
            add_paths: Vec::new(),
            add_entries: Vec::new(),
            create_dirs: Vec::new(),
            replace_entries: Vec::new(),
            output_path: Some(saved_as.to_string_lossy().to_string()),
        })
        .expect("save archive as");

        assert!(PathBuf::from(&original_path).exists());
        assert!(saved_as.exists());
        let original_info = list_archive(original_path, None).expect("list original archive");
        let saved_info =
            list_archive(saved_as.to_string_lossy().to_string(), None).expect("list saved archive");
        assert!(original_info
            .entries
            .iter()
            .any(|entry| entry.path == "original.txt"));
        assert!(saved_info
            .entries
            .iter()
            .any(|entry| entry.path == "saved.txt"));
        assert!(!saved_info
            .entries
            .iter()
            .any(|entry| entry.path == "original.txt"));
    }

    #[test]
    fn batch_extracts_archives_to_separate_folders() {
        let temp = tempdir().expect("temp dir");
        let alpha_source = temp.path().join("alpha.txt");
        let beta_source = temp.path().join("beta.txt");
        fs::write(&alpha_source, "alpha").expect("write alpha");
        fs::write(&beta_source, "beta").expect("write beta");

        let alpha = compress_archive(CompressOptions {
            source_paths: vec![alpha_source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "alpha".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress alpha");
        let beta = compress_archive(CompressOptions {
            source_paths: vec![beta_source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "beta".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress beta");

        let output_dir = temp.path().join("batch-out");
        fs::create_dir_all(&output_dir).expect("create batch output");
        let (plans, total, total_bytes) = prepare_batch_extract_plans(
            vec![alpha.output_path, beta.output_path],
            &output_dir,
            None,
        )
        .expect("prepare batch extract");
        let task = Arc::new(ExtractTask::new(
            total,
            total_bytes,
            output_dir.to_string_lossy().to_string(),
        ));

        run_batch_extract_task(
            "test-batch".to_string(),
            Arc::clone(&task),
            plans,
            None,
            ExtractConflictStrategy::Overwrite,
        );

        let state = task.state.lock().expect("read batch state");
        assert_eq!(state.status, "completed");
        assert_eq!(state.completed, state.total);
        assert_eq!(
            fs::read_to_string(output_dir.join("alpha").join("alpha.txt")).expect("read alpha"),
            "alpha"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("beta").join("beta.txt")).expect("read beta"),
            "beta"
        );
    }

    #[test]
    fn skips_existing_files_when_extract_conflict_strategy_is_skip() {
        let temp = tempdir().expect("temp dir");
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).expect("create source");
        let source = source_dir.join("same.txt");
        fs::write(&source, "from archive").expect("write source");
        let archive = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "conflict-skip".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress archive");

        let output_dir = temp.path().join("out");
        fs::create_dir_all(&output_dir).expect("create output");
        fs::write(output_dir.join("same.txt"), "existing").expect("write existing");
        let archive_path = PathBuf::from(archive.output_path);
        let entries = extract_task_entries(&archive_path, Vec::new(), None).expect("entries");
        let total = entries.len();
        let total_bytes = entries.iter().map(|entry| entry.size).sum::<u64>().max(1);
        let task = Arc::new(ExtractTask::new(
            total,
            total_bytes,
            output_dir.to_string_lossy().to_string(),
        ));

        run_extract_task(
            "test-skip".to_string(),
            task,
            archive_path,
            output_dir.clone(),
            entries,
            None,
            ExtractConflictStrategy::Skip,
        );

        assert_eq!(
            fs::read_to_string(output_dir.join("same.txt")).expect("read existing"),
            "existing"
        );
    }

    #[test]
    fn renames_existing_files_when_extract_conflict_strategy_is_rename() {
        let temp = tempdir().expect("temp dir");
        let source_dir = temp.path().join("source");
        fs::create_dir_all(&source_dir).expect("create source");
        let source = source_dir.join("same.txt");
        fs::write(&source, "from archive").expect("write source");
        let archive = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "conflict-rename".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress archive");

        let output_dir = temp.path().join("out");
        fs::create_dir_all(&output_dir).expect("create output");
        fs::write(output_dir.join("same.txt"), "existing").expect("write existing");
        let archive_path = PathBuf::from(archive.output_path);
        let entries = extract_task_entries(&archive_path, Vec::new(), None).expect("entries");
        let total = entries.len();
        let total_bytes = entries.iter().map(|entry| entry.size).sum::<u64>().max(1);
        let task = Arc::new(ExtractTask::new(
            total,
            total_bytes,
            output_dir.to_string_lossy().to_string(),
        ));

        run_extract_task(
            "test-rename".to_string(),
            task,
            archive_path,
            output_dir.clone(),
            entries,
            None,
            ExtractConflictStrategy::Rename,
        );

        assert_eq!(
            fs::read_to_string(output_dir.join("same.txt")).expect("read existing"),
            "existing"
        );
        assert_eq!(
            fs::read_to_string(output_dir.join("same 2.txt")).expect("read renamed"),
            "from archive"
        );
    }

    #[test]
    fn recognizes_expanded_archive_format_aliases() {
        for (name, expected) in [
            ("bundle.tar.gz", "TGZ"),
            ("bundle.tgz", "TGZ"),
            ("bundle.tar.bz2", "TBZ"),
            ("bundle.tbz", "TBZ"),
            ("bundle.tbz2", "TBZ"),
            ("bundle.tar.xz", "TXZ"),
            ("bundle.txz", "TXZ"),
            ("bundle.tar.zst", "ZSTD"),
            ("bundle.zstd", "ZSTD"),
            ("bundle.lha", "LHA"),
            ("bundle.lzh", "LZH"),
            ("bundle.Z", "Z"),
            ("bundle.lzma2", "LZMA2"),
            ("bundle.lz4", "LZ4"),
            ("bundle.7z.001", "7Z"),
            ("bundle.zip.001", "ZIP"),
            ("bundle.tar.gz.001", "TGZ"),
            ("bundle.part1.rar", "RAR"),
            ("bundle.z01", "ZIP"),
        ] {
            assert_eq!(archive_format(Path::new(name)), expected);
        }
    }

    #[test]
    fn guides_users_to_open_first_split_archive_volume() {
        assert_eq!(split_archive_open_error(Path::new("bundle.7z.001")), None);
        assert_eq!(
            split_archive_open_error(Path::new("bundle.part1.rar")),
            None
        );
        assert!(split_archive_open_error(Path::new("bundle.7z.002"))
            .expect("numeric split error")
            .contains(".001"));
        assert!(split_archive_open_error(Path::new("bundle.part2.rar"))
            .expect("rar split error")
            .contains(".part1.rar"));
        assert!(split_archive_open_error(Path::new("bundle.z01"))
            .expect("zip split error")
            .contains("bundle.zip"));
    }

    #[test]
    fn names_split_archive_output_folders_from_base_archive() {
        assert_eq!(
            archive_output_folder_name(Path::new("bundle.7z.001")),
            "bundle"
        );
        assert_eq!(
            archive_output_folder_name(Path::new("bundle.tar.gz.001")),
            "bundle"
        );
        assert_eq!(
            archive_output_folder_name(Path::new("bundle.part1.rar")),
            "bundle"
        );
    }

    #[test]
    fn detects_archive_entry_safety_warnings() {
        assert!(archive_entry_is_hidden(".env"));
        assert!(archive_entry_is_hidden("folder/.secret/file.txt"));
        assert!(archive_entry_is_executable("script.sh", "-rw-r--r--"));
        assert!(archive_entry_is_executable("plain.txt", "-rwxr-xr-x"));
        assert!(archive_entry_has_unsafe_path("../escape.txt"));
        assert!(archive_entry_has_unsafe_path("/tmp/escape.txt"));
        assert!(!archive_entry_has_unsafe_path("safe/folder/file.txt"));
    }

    #[test]
    fn rejects_unsafe_extract_task_entries() {
        let entries = vec![ExtractTaskEntry {
            path: "../escape.txt".to_string(),
            display_path: "../escape.txt".to_string(),
            size: 1,
            is_dir: false,
            is_unsafe_path: true,
        }];

        let error = ensure_safe_archive_entry_paths(&entries).expect_err("unsafe path");
        assert!(error.contains("不安全路径"));
    }

    #[test]
    fn treats_compressed_tar_files_as_archives() {
        for name in [
            "bundle.tar.gz",
            "bundle.tgz",
            "bundle.tar.bz2",
            "bundle.tbz",
            "bundle.tar.xz",
            "bundle.txz",
            "bundle.tar.zst",
            "bundle.tar.lz4",
        ] {
            assert!(!is_single_stream_format(Path::new(name)), "{name}");
        }

        for name in [
            "note.txt.gz",
            "note.txt.gzip",
            "note.txt.zst",
            "note.txt.lz4",
        ] {
            assert!(is_single_stream_format(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn treats_zip_container_documents_as_archives() {
        let temp = tempdir().expect("temp dir");
        let word_dir = temp.path().join("word");
        fs::create_dir_all(&word_dir).expect("create word dir");
        fs::write(word_dir.join("document.xml"), "<doc>body</doc>").expect("write document");

        let archive_path = temp.path().join("report.docx");
        run_command(
            "bsdtar",
            [
                OsStr::new("--format"),
                OsStr::new("zip"),
                OsStr::new("-cf"),
                archive_path.as_os_str(),
                OsStr::new("-C"),
                temp.path().as_os_str(),
                OsStr::new("word/document.xml"),
            ]
            .into_iter(),
            None,
        )
        .expect("create zip container document");

        assert_eq!(archive_format(&archive_path), "ZIP");
        ensure_supported_extract_format(&archive_path).expect("supported zip container");

        let info =
            list_archive(archive_path.to_string_lossy().to_string(), None).expect("list docx");
        assert_eq!(info.format, "ZIP");
        assert!(info
            .entries
            .iter()
            .any(|entry| entry.path == "word/document.xml"));

        let output_dir = temp.path().join("out");
        extract_archive(
            archive_path.to_string_lossy().to_string(),
            output_dir.to_string_lossy().to_string(),
            None,
        )
        .expect("extract docx");
        assert_eq!(
            fs::read_to_string(output_dir.join("word").join("document.xml"))
                .expect("read extracted docx entry"),
            "<doc>body</doc>"
        );
    }

    #[test]
    fn compresses_additional_tar_archive_formats_when_available() {
        for format in [
            "TGZ", "TBZ", "TXZ", "GZ", "BZ2", "XZ", "Z", "ZSTD", "LZMA", "LZMA2", "LZ4",
        ] {
            let temp = tempdir().expect("temp dir");
            let source = temp.path().join("hello.txt");
            fs::write(&source, format!("hello {format}")).expect("write source");

            let result = compress_archive(CompressOptions {
                source_paths: vec![source.to_string_lossy().to_string()],
                output_dir: temp.path().to_string_lossy().to_string(),
                archive_name: format!("bundle-{format}"),
                format: format.to_string(),
                compression_level: None,
                password: None,
                volume_size_mb: None,
                excluded_paths: Vec::new(),
                skip_ds_store: false,
                advanced: CompressAdvancedOptions::default(),
            });

            let Ok(result) = result else {
                continue;
            };

            let info = list_archive(result.output_path.clone(), None).expect("list archive");
            assert_eq!(info.file_count, 1);
            assert!(info.entries.iter().any(|entry| entry.name == "hello.txt"));

            let output_dir = temp.path().join("out");
            extract_archive(
                result.output_path,
                output_dir.to_string_lossy().to_string(),
                None,
            )
            .expect("extract archive");

            assert_eq!(
                fs::read_to_string(output_dir.join("hello.txt")).expect("read extracted"),
                format!("hello {format}")
            );
        }
    }

    #[test]
    fn compresses_zip_with_password() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("secret.txt");
        fs::write(&source, "zip password").expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "secret".to_string(),
            format: "ZIP".to_string(),
            compression_level: Some(9),
            password: Some("packo-pass".to_string()),
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress encrypted zip");

        let info = list_archive(result.output_path.clone(), Some("packo-pass".to_string()))
            .expect("list encrypted zip");
        assert!(info.entries.iter().any(|entry| entry.name == "secret.txt"));

        let output_dir = temp.path().join("out");
        extract_archive(
            result.output_path,
            output_dir.to_string_lossy().to_string(),
            Some("packo-pass".to_string()),
        )
        .expect("extract encrypted zip");

        assert_eq!(
            fs::read_to_string(output_dir.join("secret.txt")).expect("read extracted"),
            "zip password"
        );
    }

    #[test]
    fn encrypted_zip_without_password_fails_quickly() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("secret.txt");
        fs::write(&source, "zip password").expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "secret-no-password".to_string(),
            format: "ZIP".to_string(),
            compression_level: Some(9),
            password: Some("packo-pass".to_string()),
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress encrypted zip");

        let output_dir = temp.path().join("out-without-password");
        let started = Instant::now();
        let error = extract_archive(
            result.output_path,
            output_dir.to_string_lossy().to_string(),
            None,
        )
        .expect_err("extract should require password");

        assert!(
            started.elapsed() < Duration::from_secs(2),
            "password failure should not wait for interactive input"
        );
        assert!(
            error.to_lowercase().contains("passphrase") || error.contains("密码"),
            "unexpected password error: {error}"
        );
    }

    #[test]
    fn compresses_7z_with_password() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("secret.txt");
        fs::write(&source, "7z password").expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "secret-7z".to_string(),
            format: "7Z".to_string(),
            compression_level: Some(9),
            password: Some("packo-pass".to_string()),
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress encrypted 7z");

        assert!(sevenz_rust2::ArchiveReader::open(
            &result.output_path,
            sevenz_rust2::Password::empty()
        )
        .is_err());

        let mut reader = sevenz_rust2::ArchiveReader::open(
            &result.output_path,
            sevenz_rust2::Password::new("packo-pass"),
        )
        .expect("open encrypted 7z");
        let body = reader.read_file("secret.txt").expect("read encrypted 7z");
        assert_eq!(String::from_utf8(body).expect("utf8 body"), "7z password");
    }

    #[test]
    fn compresses_split_zip_archive() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("large.bin");
        fs::write(&source, vec![42_u8; 2 * 1024 * 1024]).expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "split".to_string(),
            format: "ZIP".to_string(),
            compression_level: Some(0),
            password: None,
            volume_size_mb: Some(1),
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress split zip");

        let zip_path = PathBuf::from(result.output_path);
        assert!(zip_path.exists());
        assert!(zip_path.with_extension("z01").exists());
    }

    #[test]
    fn lists_and_extracts_split_zip_from_main_or_z_part() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("large-random.bin");
        let mut seed = 0xfeed_beefu32;
        let data: Vec<u8> = (0..(2 * 1024 * 1024))
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        fs::write(&source, &data).expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "split-readable".to_string(),
            format: "ZIP".to_string(),
            compression_level: Some(0),
            password: None,
            volume_size_mb: Some(1),
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress split zip");

        let zip_path = PathBuf::from(&result.output_path);
        let z01_path = zip_path.with_extension("z01");
        assert!(z01_path.exists());

        let info = list_archive(result.output_path.clone(), None).expect("list split zip");
        assert_eq!(info.path, result.output_path);
        assert!(info.properties.split.is_split);
        assert!(info
            .entries
            .iter()
            .any(|entry| entry.path == "large-random.bin"));
        test_archive_integrity(result.output_path.clone(), None).expect("test split zip");

        let z01_info =
            list_archive(z01_path.to_string_lossy().to_string(), None).expect("list from z01");
        assert_eq!(z01_info.path, result.output_path);
        assert!(z01_info.properties.split.is_split);

        let output_dir = temp.path().join("out-main");
        extract_archive(
            result.output_path.clone(),
            output_dir.to_string_lossy().to_string(),
            None,
        )
        .expect("extract split zip from main");
        assert_eq!(
            fs::read(output_dir.join("large-random.bin")).expect("read extracted main"),
            data
        );

        let z_part_output_dir = temp.path().join("out-z01");
        extract_archive(
            z01_path.to_string_lossy().to_string(),
            z_part_output_dir.to_string_lossy().to_string(),
            None,
        )
        .expect("extract split zip from z01");
        assert_eq!(
            fs::read(z_part_output_dir.join("large-random.bin")).expect("read extracted z01"),
            data
        );
    }

    #[test]
    fn compresses_split_7z_archive() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("large-random.bin");
        let mut seed = 0x1234_5678_u32;
        let data: Vec<u8> = (0..(2 * 1024 * 1024))
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        fs::write(&source, data).expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "split-7z".to_string(),
            format: "7Z".to_string(),
            compression_level: Some(0),
            password: None,
            volume_size_mb: Some(1),
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress split 7z");

        let first_volume = PathBuf::from(result.output_path);
        assert!(first_volume.exists());
        assert!(first_volume
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".7z.001")));
        assert!(first_volume.with_extension("002").exists());
        assert!(!temp.path().join(".split-7z.packo.7z").exists());
    }

    #[test]
    fn batch_compresses_top_level_sources_separately() {
        let temp = tempdir().expect("temp dir");
        let alpha = temp.path().join("alpha.txt");
        let beta = temp.path().join("beta.txt");
        fs::write(&alpha, "alpha").expect("write alpha");
        fs::write(&beta, "beta").expect("write beta");

        let result = compress_archive(CompressOptions {
            source_paths: vec![
                alpha.to_string_lossy().to_string(),
                beta.to_string_lossy().to_string(),
            ],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "ignored".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions {
                batch_queue: true,
                ..CompressAdvancedOptions::default()
            },
        })
        .expect("batch compress");

        assert_eq!(PathBuf::from(result.output_path), temp.path());
        assert!(temp.path().join("alpha.zip").exists());
        assert!(temp.path().join("beta.zip").exists());
        assert!(!temp.path().join("ignored.zip").exists());
    }

    #[test]
    fn compresses_7z_with_advanced_settings_without_password() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("advanced.txt");
        fs::write(&source, "advanced 7z").expect("write source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "advanced-7z".to_string(),
            format: "7Z".to_string(),
            compression_level: Some(5),
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions {
                dictionary_size_mb: Some(1),
                solid: Some(true),
                threads: Some(2),
                method: Some("LZMA2".to_string()),
                ..CompressAdvancedOptions::default()
            },
        })
        .expect("compress advanced 7z");

        let mut reader =
            sevenz_rust2::ArchiveReader::open(&result.output_path, sevenz_rust2::Password::empty())
                .expect("open advanced 7z");
        let body = reader.read_file("advanced.txt").expect("read advanced 7z");
        assert_eq!(String::from_utf8(body).expect("utf8 body"), "advanced 7z");
    }

    #[test]
    fn compresses_folder_with_excluded_child_path() {
        let temp = tempdir().expect("temp dir");
        let folder = temp.path().join("folder");
        fs::create_dir_all(&folder).expect("create folder");
        fs::write(folder.join("keep.txt"), "keep").expect("write keep");
        fs::write(folder.join("skip.txt"), "skip").expect("write skip");

        let result = compress_archive(CompressOptions {
            source_paths: vec![folder.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "excluded".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: vec![folder.join("skip.txt").to_string_lossy().to_string()],
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress archive");

        let info = list_archive(result.output_path, None).expect("list archive");
        assert!(info
            .entries
            .iter()
            .any(|entry| entry.path == "folder/keep.txt"));
        assert!(!info
            .entries
            .iter()
            .any(|entry| entry.path == "folder/skip.txt"));
    }

    #[test]
    fn skips_ds_store_when_enabled() {
        let temp = tempdir().expect("temp dir");
        let folder = temp.path().join("folder");
        let nested = folder.join("nested");
        fs::create_dir_all(&nested).expect("create folders");
        fs::write(folder.join("keep.txt"), "keep").expect("write keep");
        fs::write(folder.join(".DS_Store"), "metadata").expect("write root ds store");
        fs::write(nested.join(".DS_Store"), "nested metadata").expect("write nested ds store");

        let described = describe_compress_paths(
            vec![folder.to_string_lossy().to_string()],
            Vec::new(),
            Some(true),
            Some(false),
        )
        .expect("describe compress paths");
        assert!(described.iter().any(|item| item.name == "keep.txt"));
        assert!(!described
            .iter()
            .any(|item| item.name.eq_ignore_ascii_case(".DS_Store")));

        let result = compress_archive(CompressOptions {
            source_paths: vec![folder.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "no-ds-store".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: true,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress archive");

        let info = list_archive(result.output_path, None).expect("list archive");
        assert!(info
            .entries
            .iter()
            .any(|entry| entry.path == "folder/keep.txt"));
        assert!(!info
            .entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(".DS_Store")));
    }

    #[test]
    fn skips_macos_metadata_when_enabled() {
        let temp = tempdir().expect("temp dir");
        let folder = temp.path().join("folder");
        let macosx = folder.join("__MACOSX");
        fs::create_dir_all(&macosx).expect("create metadata folder");
        fs::write(folder.join("keep.txt"), "keep").expect("write keep");
        fs::write(folder.join("._keep.txt"), "resource fork").expect("write resource fork");
        fs::write(macosx.join("extra"), "metadata").expect("write metadata");

        let described = describe_compress_paths(
            vec![folder.to_string_lossy().to_string()],
            Vec::new(),
            Some(false),
            Some(true),
        )
        .expect("describe compress paths");
        assert!(described.iter().any(|item| item.name == "keep.txt"));
        assert!(!described.iter().any(|item| item.name == "__MACOSX"));
        assert!(!described.iter().any(|item| item.name.starts_with("._")));

        let result = compress_archive(CompressOptions {
            source_paths: vec![folder.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "no-macos-metadata".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions {
                skip_macos_metadata: true,
                ..CompressAdvancedOptions::default()
            },
        })
        .expect("compress archive");

        let info = list_archive(result.output_path, None).expect("list archive");
        assert!(info
            .entries
            .iter()
            .any(|entry| entry.path == "folder/keep.txt"));
        assert!(!info
            .entries
            .iter()
            .any(|entry| { entry.path.contains("__MACOSX") || entry.name.starts_with("._") }));
    }

    #[test]
    fn extracts_selected_archive_entries_only() {
        let temp = tempdir().expect("temp dir");
        let keep = temp.path().join("keep.txt");
        let skip = temp.path().join("skip.txt");
        fs::write(&keep, "keep").expect("write keep");
        fs::write(&skip, "skip").expect("write skip");

        let result = compress_archive(CompressOptions {
            source_paths: vec![
                keep.to_string_lossy().to_string(),
                skip.to_string_lossy().to_string(),
            ],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "selected".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress archive");

        let output_dir = temp.path().join("selected-out");
        extract_archive_entries(
            result.output_path,
            output_dir.to_string_lossy().to_string(),
            vec!["keep.txt".to_string()],
            None,
        )
        .expect("extract selected");

        assert_eq!(
            fs::read_to_string(output_dir.join("keep.txt")).expect("read selected"),
            "keep"
        );
        assert!(!output_dir.join("skip.txt").exists());
    }

    #[test]
    fn extracts_promised_entry_to_target_file_name() {
        let temp = tempdir().expect("temp dir");
        let nested_dir = temp.path().join("nested");
        fs::create_dir_all(&nested_dir).expect("create nested dir");
        fs::write(nested_dir.join("drag.txt"), "drag body").expect("write nested source");

        let archive_path = temp.path().join("nested.zip");
        run_command(
            "bsdtar",
            [
                OsStr::new("--format"),
                OsStr::new("zip"),
                OsStr::new("-cf"),
                archive_path.as_os_str(),
                OsStr::new("-C"),
                temp.path().as_os_str(),
                OsStr::new("nested/drag.txt"),
            ]
            .into_iter(),
            None,
        )
        .expect("create nested archive");

        let output_path = temp.path().join("out").join("drag.txt");
        extract_archive_entry_to_promised_file(
            &archive_path,
            "nested/drag.txt",
            &output_path,
            None,
        )
        .expect("extract promised entry");

        assert_eq!(
            fs::read_to_string(&output_path).expect("read promised file"),
            "drag body"
        );
        assert!(!temp.path().join("out").join("nested").exists());
    }

    #[test]
    fn extracts_promised_folder_to_target_folder_name() {
        let temp = tempdir().expect("temp dir");
        let nested_dir = temp.path().join("nested").join("deeper");
        fs::create_dir_all(&nested_dir).expect("create nested dir");
        fs::write(temp.path().join("nested").join("drag.txt"), "drag body")
            .expect("write nested source");
        fs::write(nested_dir.join("more.txt"), "more body").expect("write deep source");

        let archive_path = temp.path().join("nested-folder.zip");
        run_command(
            "bsdtar",
            [
                OsStr::new("--format"),
                OsStr::new("zip"),
                OsStr::new("-cf"),
                archive_path.as_os_str(),
                OsStr::new("-C"),
                temp.path().as_os_str(),
                OsStr::new("nested/drag.txt"),
                OsStr::new("nested/deeper/more.txt"),
            ]
            .into_iter(),
            None,
        )
        .expect("create nested archive");

        let output_path = temp.path().join("out").join("Dragged Folder");
        extract_archive_entry_to_promised_item(&archive_path, "nested", &output_path, true, None)
            .expect("extract promised folder");

        assert_eq!(
            fs::read_to_string(output_path.join("drag.txt")).expect("read promised file"),
            "drag body"
        );
        assert_eq!(
            fs::read_to_string(output_path.join("deeper").join("more.txt"))
                .expect("read promised deep file"),
            "more body"
        );
        assert!(!output_path.join("nested").exists());
    }

    #[cfg(unix)]
    #[test]
    fn normalizes_temp_permissions_before_promised_copy() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().expect("temp dir");
        let source_root = temp.path().join("extracted");
        let restricted_dir = source_root.join("nested");
        fs::create_dir_all(&restricted_dir).expect("create restricted dir");
        let restricted_file = restricted_dir.join("locked.txt");
        fs::write(&restricted_file, "locked body").expect("write restricted file");

        let mut file_permissions = fs::metadata(&restricted_file)
            .expect("restricted file metadata")
            .permissions();
        file_permissions.set_mode(0o000);
        fs::set_permissions(&restricted_file, file_permissions).expect("restrict file");

        let mut dir_permissions = fs::metadata(&restricted_dir)
            .expect("restricted dir metadata")
            .permissions();
        dir_permissions.set_mode(0o000);
        fs::set_permissions(&restricted_dir, dir_permissions).expect("restrict dir");

        ensure_copyable_temp_tree_permissions(&source_root).expect("normalize temp permissions");

        let output_path = temp.path().join("out").join("nested");
        copy_path_recursive(&restricted_dir, &output_path).expect("copy normalized tree");

        assert_eq!(
            fs::read_to_string(output_path.join("locked.txt")).expect("read copied file"),
            "locked body"
        );
    }

    #[test]
    fn previews_archive_entry_to_temp_file() {
        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("preview.txt");
        fs::write(&source, "preview body").expect("write preview source");

        let result = compress_archive(CompressOptions {
            source_paths: vec![source.to_string_lossy().to_string()],
            output_dir: temp.path().to_string_lossy().to_string(),
            archive_name: "preview-source".to_string(),
            format: "ZIP".to_string(),
            compression_level: None,
            password: None,
            volume_size_mb: None,
            excluded_paths: Vec::new(),
            skip_ds_store: false,
            advanced: CompressAdvancedOptions::default(),
        })
        .expect("compress archive");

        let preview = preview_archive_entry(result.output_path, "preview.txt".to_string(), None)
            .expect("preview archive entry");

        assert!(preview.output_path.contains("cardiganzip-preview"));
        assert_eq!(
            fs::read_to_string(preview.output_path).expect("read preview"),
            "preview body"
        );
    }

    #[test]
    fn extracts_single_file_compression_streams() {
        for (format, program, extension, args) in [
            ("GZ", "gzip", "gz", vec!["-c"]),
            ("GZIP", "gzip", "gzip", vec!["-c"]),
            ("BZ2", "bzip2", "bz2", vec!["-c"]),
            ("BZIP2", "bzip2", "bzip2", vec!["-c"]),
            ("XZ", "xz", "xz", vec!["-c"]),
            ("ZSTD", "zstd", "zst", vec!["-q", "-c"]),
            ("LZMA", "lzma", "lzma", vec!["-c"]),
            ("LZMA2", "xz", "lzma2", vec!["-c"]),
            ("LZ4", "lz4", "lz4", vec!["-q", "-c"]),
        ] {
            if run_command(program, [OsStr::new("--version")].into_iter(), None).is_err() {
                continue;
            }

            let temp = tempdir().expect("temp dir");
            let source = temp.path().join("note.txt");
            fs::write(&source, format!("stream {format}")).expect("write source");

            let stream_path = temp.path().join(format!("note.txt.{extension}"));
            let mut command_args: Vec<OsString> = args.into_iter().map(OsString::from).collect();
            command_args.push(source.as_os_str().to_os_string());
            let compressed = run_command_bytes(
                program,
                command_args.iter().map(|arg| arg.as_os_str()),
                None,
            )
            .expect("compress stream");
            fs::write(&stream_path, compressed).expect("write stream");

            let info =
                list_archive(stream_path.to_string_lossy().to_string(), None).expect("list stream");
            assert_eq!(info.format, format);
            assert_eq!(info.file_count, 1);
            assert_eq!(info.entries[0].name, "note.txt");

            let output_dir = temp.path().join("out");
            extract_archive(
                stream_path.to_string_lossy().to_string(),
                output_dir.to_string_lossy().to_string(),
                None,
            )
            .expect("extract stream");

            assert_eq!(
                fs::read_to_string(output_dir.join("note.txt")).expect("read extracted stream"),
                format!("stream {format}")
            );
        }

        let temp = tempdir().expect("temp dir");
        let source = temp.path().join("note.txt");
        fs::write(&source, "stream Z").expect("write source");
        let stream_path = temp.path().join("note.txt.Z");
        if let Ok(compressed) = run_command_bytes(
            "compress",
            [OsStr::new("-c"), source.as_os_str()].into_iter(),
            None,
        ) {
            fs::write(&stream_path, compressed).expect("write stream");

            let info =
                list_archive(stream_path.to_string_lossy().to_string(), None).expect("list stream");
            assert_eq!(info.format, "Z");
            assert_eq!(info.file_count, 1);
            assert_eq!(info.entries[0].name, "note.txt");

            let output_dir = temp.path().join("out");
            extract_archive(
                stream_path.to_string_lossy().to_string(),
                output_dir.to_string_lossy().to_string(),
                None,
            )
            .expect("extract stream");

            assert_eq!(
                fs::read_to_string(output_dir.join("note.txt")).expect("read extracted stream"),
                "stream Z"
            );
        }
    }
}
