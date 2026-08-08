//! Platform glue.
//!
//! Currently just the macOS "Open With" / double-click handler: Finder delivers
//! the opened file as an Apple Event (`kAEOpenDocuments`), not on argv, and it
//! can arrive before or after the window exists. We register a handler on the
//! shared `NSAppleEventManager` that buffers opened paths into a queue; the iced
//! app drains it via `take_open_files` on a timer subscription. Mirrors the
//! Python `PhotoViewerApplication.event` + `_pending_path` buffering.
//!
//! Timing gotcha: `-[NSApplication finishLaunching]` installs AppKit's own
//! `odoc` handler, which overwrites any we set beforehand — so registering in
//! `main()` (before winit runs the app) silently loses the event, and the open
//! Apple Event times out ("The document could not be opened"). We therefore
//! defer the `NSAppleEventManager` registration to
//! `NSApplicationDidFinishLaunchingNotification`, which fires *after* AppKit's
//! own install, so ours wins. The pending launch-time `odoc` is only dispatched
//! on the next run-loop pass, after that notification, so this also catches the
//! double-click-to-launch case.

use std::future::Future;
use std::path::PathBuf;

use crate::core::library::IMAGE_EXTENSIONS;

/// Register any platform open-file hooks. Call once, before the event loop
/// starts. No-op off macOS.
pub fn install_open_file_handler() {
    #[cfg(target_os = "macos")]
    macos::install();
}

/// Pay the native file panel's one-off startup cost now, so the first `o` does
/// not. Must be called on the main thread; no-op off macOS.
///
/// AppKit runs the open/save panel out of process, in
/// `com.apple.appkit.xpc.openAndSavePanelService`. The first `NSOpenPanel` of a
/// process launches that service and waits for it: measured at ~0.8s, against
/// ~0.1s for every one after it. That whole second lands on the main thread —
/// the app is frozen for it — so it is spent once at startup, when nothing is
/// waiting, rather than the first time the user asks to open a file.
pub fn prewarm_file_dialog() {
    #[cfg(target_os = "macos")]
    macos::prewarm_file_dialog();
}

/// Ask the user for something to open: a photograph, or a folder of them.
///
/// One panel answers both, because [`crate::app::App::open`] takes both — a
/// folder lands on the wall, a file in the single view. rfd cannot express that
/// (its panels choose files or folders, never both), hence the panel built by
/// hand on macOS; elsewhere it falls back to rfd's file-only picker.
///
/// Must be called from the main thread — panels are main-thread-only, and iced
/// runs `update` there. The panel goes up as this is called; the future it hands
/// back resolves when the user closes it.
pub fn pick_open_target(dir: Option<PathBuf>) -> impl Future<Output = Option<PathBuf>> + Send {
    #[cfg(target_os = "macos")]
    {
        let answer = macos::pick_open_target(dir, &IMAGE_EXTENSIONS);
        // The sender is dropped without a value only if the panel never opened.
        async move { answer.await.unwrap_or(None) }
    }
    #[cfg(not(target_os = "macos"))]
    {
        async move {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_title("Select an image to open")
                .add_filter("Images", &IMAGE_EXTENSIONS);
            if let Some(dir) = dir {
                dialog = dialog.set_directory(dir);
            }
            dialog
                .pick_file()
                .await
                .map(|handle| handle.path().to_path_buf())
        }
    }
}

/// Take (and clear) the paths the platform has delivered since the last call.
/// Always empty off macOS.
pub fn take_open_files() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos::take()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSModalResponse, NSModalResponseOK, NSOpenPanel};
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleEventManager, NSArray, NSData, NSNotificationCenter,
        NSString, NSURL,
    };
    use tokio::sync::oneshot;

    /// FourCharCode (OSType) from a 4-byte tag, e.g. `b"odoc"`.
    const fn fourcc(tag: &[u8; 4]) -> u32 {
        ((tag[0] as u32) << 24) | ((tag[1] as u32) << 16) | ((tag[2] as u32) << 8) | (tag[3] as u32)
    }

    const K_CORE_EVENT_CLASS: u32 = fourcc(b"aevt"); // kCoreEventClass
    const K_AE_OPEN_DOCUMENTS: u32 = fourcc(b"odoc"); // kAEOpenDocuments
    const KEY_DIRECT_OBJECT: u32 = fourcc(b"----"); // keyDirectObject
    const TYPE_FILE_URL: u32 = fourcc(b"furl"); // typeFileURL

    /// Paths delivered by Finder, awaiting the app to drain them.
    static QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; we hold no ivars and
        // don't implement Drop.
        #[unsafe(super(NSObject))]
        #[name = "PVOpenFileHandler"]
        #[ivars = ()]
        struct Handler;

        impl Handler {
            #[unsafe(method(handleAppleEvent:withReplyEvent:))]
            fn handle_apple_event(
                &self,
                event: &NSAppleEventDescriptor,
                _reply: &NSAppleEventDescriptor,
            ) {
                let paths = extract_file_paths(event);
                if !paths.is_empty() {
                    if let Ok(mut queue) = QUEUE.lock() {
                        queue.extend(paths);
                    }
                }
            }

            // Fires after AppKit has installed its own odoc handler in
            // finishLaunching; now safe to register ours on top.
            #[unsafe(method(applicationDidFinishLaunching:))]
            fn application_did_finish_launching(&self, _note: &NSObject) {
                self.register_apple_event_handler();
            }
        }
    );

    impl Handler {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            unsafe { msg_send![super(this), init] }
        }

        /// Register self as the shared manager's `kAEOpenDocuments` handler.
        /// Must run *after* `NSApplication` finishLaunching, else AppKit's own
        /// registration overwrites ours.
        fn register_apple_event_handler(&self) {
            let manager = NSAppleEventManager::sharedAppleEventManager();
            unsafe {
                manager.setEventHandler_andSelector_forEventClass_andEventID(
                    self,
                    sel!(handleAppleEvent:withReplyEvent:),
                    K_CORE_EVENT_CLASS,
                    K_AE_OPEN_DOCUMENTS,
                );
            }
        }
    }

    /// Pull the file paths out of a `kAEOpenDocuments` event: its direct object
    /// is a list of file references; coerce each to a `typeFileURL` and read the
    /// URL back into a filesystem path via `NSURL`.
    fn extract_file_paths(event: &NSAppleEventDescriptor) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Some(direct) = event.paramDescriptorForKeyword(KEY_DIRECT_OBJECT) else {
            return out;
        };
        let count = direct.numberOfItems();
        for i in 1..=count {
            let Some(item) = direct.descriptorAtIndex(i) else {
                continue;
            };
            let Some(url_desc) = item.coerceToDescriptorType(TYPE_FILE_URL) else {
                continue;
            };
            if let Some(path) = file_url_data_to_path(&url_desc.data()) {
                out.push(path);
            }
        }
        out
    }

    /// A `typeFileURL` descriptor's data is a UTF-8 `file://` URL; decode it to a
    /// path with `NSURL` (which handles percent-decoding).
    fn file_url_data_to_path(data: &NSData) -> Option<PathBuf> {
        let len = data.length();
        if len == 0 {
            return None;
        }
        // NSData exposes only `length` in objc2-foundation; read `bytes` directly.
        let ptr: *const core::ffi::c_void = unsafe { msg_send![data, bytes] };
        if ptr.is_null() {
            return None;
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
        let url_string = String::from_utf8_lossy(bytes);
        let url = NSURL::URLWithString(&NSString::from_str(&url_string))?;
        let path = url.path()?;
        Some(PathBuf::from(path.to_string()))
    }

    pub(super) fn install() {
        let handler = Handler::new();

        // Register the Apple Event handler only once AppKit has finished
        // launching (see module docs): observe the notification and install
        // from there. addObserver keeps only a weak reference to the observer,
        // and setEventHandler likewise, so leak the handler to keep it alive
        // for the whole process.
        unsafe {
            let center = NSNotificationCenter::defaultCenter();
            center.addObserver_selector_name_object(
                &handler,
                sel!(applicationDidFinishLaunching:),
                Some(&NSString::from_str(
                    "NSApplicationDidFinishLaunchingNotification",
                )),
                None,
            );
        }
        std::mem::forget(handler);
    }

    // The panel built at startup, kept alive for the life of the process.
    //
    // Building one is what launches the panel service; holding it is what keeps
    // the service from idling out again, which would put the whole cost back on
    // the next open. It is never shown — rfd builds its own panel, and only
    // inherits the warm service.
    thread_local! {
        static PREWARMED: RefCell<Option<Retained<NSOpenPanel>>> = const { RefCell::new(None) };
    }

    pub(super) fn prewarm_file_dialog() {
        // Panels are main-thread-only, and this is called from `update`, which
        // iced runs there. Off it, skip: a cold first open is the worse of the
        // two outcomes but far from the worst.
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        PREWARMED.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(NSOpenPanel::openPanel(mtm));
            }
        });
    }

    /// Put up an open panel that takes an image *or* a folder, and hand back
    /// the answer down a channel.
    ///
    /// `setCanChooseDirectories` is the whole point: with it a folder is a
    /// thing the panel can return, not only somewhere to descend into. Files
    /// outside `extensions` are greyed out; folders stay pickable regardless.
    pub(super) fn pick_open_target(
        dir: Option<PathBuf>,
        extensions: &[&str],
    ) -> oneshot::Receiver<Option<PathBuf>> {
        let (tx, rx) = oneshot::channel();
        // Panels are main-thread-only. Off it there is nothing to show, so
        // answer as a cancelled panel would.
        let Some(mtm) = MainThreadMarker::new() else {
            let _ = tx.send(None);
            return rx;
        };

        let panel = NSOpenPanel::openPanel(mtm);
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(true);
        panel.setAllowsMultipleSelection(false);
        // Modern panels don't draw their title bar, so the message is the only
        // line the user actually reads.
        panel.setMessage(Some(&NSString::from_str(
            "Select an image, or a folder of images",
        )));
        let types: Vec<_> = extensions.iter().map(|e| NSString::from_str(e)).collect();
        // Superseded by allowedContentTypes, which would pull in
        // objc2-uniform-type-identifiers to say the same three extensions.
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
        if let Some(dir) = dir.as_deref().and_then(|d| d.to_str()) {
            let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(dir), true);
            panel.setDirectoryURL(Some(&url));
        }

        // The block's type says it may run more than once (it will not), so the
        // sender has to be taken out from behind a cell rather than moved.
        let sender = RefCell::new(Some(tx));
        let answering = panel.clone();
        let handler = RcBlock::new(move |response: NSModalResponse| {
            let picked = (response == NSModalResponseOK)
                .then(|| answering.URLs().iter().next().and_then(|url| url.path()))
                .flatten()
                .map(|path| PathBuf::from(path.to_string()));
            if let Some(tx) = sender.borrow_mut().take() {
                let _ = tx.send(picked);
            }
        });

        // A sheet on the app's window, matching the destination panel (rfd puts
        // that one up the same way). Windowless — nothing is on screen yet — it
        // has to stand on its own.
        let app = NSApplication::sharedApplication(mtm);
        let window = app.mainWindow().or_else(|| app.windows().firstObject());
        match window {
            Some(window) => panel.beginSheetModalForWindow_completionHandler(&window, &handler),
            None => panel.beginWithCompletionHandler(&handler),
        }
        rx
    }

    pub(super) fn take() -> Vec<PathBuf> {
        match QUEUE.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(_) => Vec::new(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use objc2_foundation::NSData;

        /// Build a real `kAEOpenDocuments` event carrying one file URL and check
        /// we pull the (percent-decoded) filesystem path back out. Exercises the
        /// descriptor walk + `NSURL` decoding, the parts most likely to be wrong.
        #[test]
        fn extracts_path_from_odoc_event() {
            // A space forces percent-encoding in the URL, so a passing test also
            // proves the decode round-trips.
            let path = "/private/tmp/pv ae test.png";
            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
            let abs = url.absoluteString().expect("absoluteString").to_string();
            let data = NSData::with_bytes(abs.as_bytes());
            let furl = NSAppleEventDescriptor::descriptorWithDescriptorType_data(
                TYPE_FILE_URL,
                Some(&data),
            )
            .expect("furl descriptor");

            let list = NSAppleEventDescriptor::listDescriptor();
            list.insertDescriptor_atIndex(&furl, 1);

            let event = NSAppleEventDescriptor::appleEventWithEventClass_eventID_targetDescriptor_returnID_transactionID(
                K_CORE_EVENT_CLASS,
                K_AE_OPEN_DOCUMENTS,
                None,
                -1, // kAutoGenerateReturnID
                0,  // kAnyTransactionID
            );
            event.setParamDescriptor_forKeyword(&list, KEY_DIRECT_OBJECT);

            assert_eq!(extract_file_paths(&event), vec![PathBuf::from(path)]);
        }
    }
}
