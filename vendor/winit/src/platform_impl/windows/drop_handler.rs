use std::ffi::{c_void, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::core::{IUnknown, GUID, HRESULT};
use windows_sys::Win32::Foundation::{DV_E_FORMATETC, HGLOBAL, HWND, POINTL, S_OK};
use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};
use windows_sys::Win32::System::Com::{
    IDataObject, IStream, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{
    ReleaseStgMedium, CF_HDROP, DROPEFFECT_COPY, DROPEFFECT_NONE,
};
use windows_sys::Win32::UI::Shell::{
    DragFinish, DragQueryFileW, FILEDESCRIPTORA, FILEDESCRIPTORW, FILEGROUPDESCRIPTORA,
    FILEGROUPDESCRIPTORW, HDROP,
};

use tracing::debug;

use crate::platform_impl::platform::definitions::{
    IDataObjectVtbl, IDropTarget, IDropTargetVtbl, IUnknownVtbl,
};
use crate::platform_impl::platform::WindowId;

use crate::event::Event;
use crate::window::WindowId as RootWindowId;

#[repr(C)]
pub struct FileDropHandlerData {
    pub interface: IDropTarget,
    refcount: AtomicUsize,
    window: HWND,
    send_event: Box<dyn Fn(Event<()>)>,
    cursor_effect: u32,
    hovered_is_valid: bool, /* If the currently hovered item is not valid there must not be any
                             * `HoveredFileCancelled` emitted */
}

pub struct FileDropHandler {
    pub data: *mut FileDropHandlerData,
}

#[allow(non_snake_case)]
impl FileDropHandler {
    pub fn new(window: HWND, send_event: Box<dyn Fn(Event<()>)>) -> FileDropHandler {
        let data = Box::new(FileDropHandlerData {
            interface: IDropTarget { lpVtbl: &DROP_TARGET_VTBL as *const IDropTargetVtbl },
            refcount: AtomicUsize::new(1),
            window,
            send_event,
            cursor_effect: DROPEFFECT_NONE,
            hovered_is_valid: false,
        });
        FileDropHandler { data: Box::into_raw(data) }
    }

    // Implement IUnknown
    pub unsafe extern "system" fn QueryInterface(
        _this: *mut IUnknown,
        _riid: *const GUID,
        _ppvObject: *mut *mut c_void,
    ) -> HRESULT {
        // This function doesn't appear to be required for an `IDropTarget`.
        // An implementation would be nice however.
        unimplemented!();
    }

    pub unsafe extern "system" fn AddRef(this: *mut IUnknown) -> u32 {
        let drop_handler_data = unsafe { Self::from_interface(this) };
        let count = drop_handler_data.refcount.fetch_add(1, Ordering::Release) + 1;
        count as u32
    }

    pub unsafe extern "system" fn Release(this: *mut IUnknown) -> u32 {
        let drop_handler = unsafe { Self::from_interface(this) };
        let count = drop_handler.refcount.fetch_sub(1, Ordering::Release) - 1;
        if count == 0 {
            // Destroy the underlying data
            drop(unsafe { Box::from_raw(drop_handler as *mut FileDropHandlerData) });
        }
        count as u32
    }

    pub unsafe extern "system" fn DragEnter(
        this: *mut IDropTarget,
        pDataObj: *const IDataObject,
        _grfKeyState: u32,
        _pt: *const POINTL,
        pdwEffect: *mut u32,
    ) -> HRESULT {
        use crate::event::WindowEvent::HoveredFile;
        let drop_handler = unsafe { Self::from_interface(this) };
        let hdrop = unsafe {
            Self::iterate_filenames(pDataObj, |filename| {
                drop_handler.send_event(Event::WindowEvent {
                    window_id: RootWindowId(WindowId(drop_handler.window)),
                    event: HoveredFile(filename),
                });
            })
        };
        // PdfCraft patch: files without a path on disk (Outlook attachments) hover by name.
        let virtual_names = if hdrop.is_none() { unsafe { virtual_file_names(pDataObj) } } else { None };
        for name in virtual_names.iter().flatten() {
            drop_handler.send_event(Event::WindowEvent {
                window_id: RootWindowId(WindowId(drop_handler.window)),
                event: HoveredFile(PathBuf::from(name)),
            });
        }
        drop_handler.hovered_is_valid = hdrop.is_some() || virtual_names.is_some();
        drop_handler.cursor_effect =
            if drop_handler.hovered_is_valid { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        unsafe {
            *pdwEffect = drop_handler.cursor_effect;
        }

        S_OK
    }

    pub unsafe extern "system" fn DragOver(
        this: *mut IDropTarget,
        _grfKeyState: u32,
        _pt: *const POINTL,
        pdwEffect: *mut u32,
    ) -> HRESULT {
        let drop_handler = unsafe { Self::from_interface(this) };
        unsafe {
            *pdwEffect = drop_handler.cursor_effect;
        }

        S_OK
    }

    pub unsafe extern "system" fn DragLeave(this: *mut IDropTarget) -> HRESULT {
        use crate::event::WindowEvent::HoveredFileCancelled;
        let drop_handler = unsafe { Self::from_interface(this) };
        if drop_handler.hovered_is_valid {
            drop_handler.send_event(Event::WindowEvent {
                window_id: RootWindowId(WindowId(drop_handler.window)),
                event: HoveredFileCancelled,
            });
        }

        S_OK
    }

    pub unsafe extern "system" fn Drop(
        this: *mut IDropTarget,
        pDataObj: *const IDataObject,
        _grfKeyState: u32,
        _pt: *const POINTL,
        _pdwEffect: *mut u32,
    ) -> HRESULT {
        use crate::event::WindowEvent::DroppedFile;
        let drop_handler = unsafe { Self::from_interface(this) };
        let hdrop = unsafe {
            Self::iterate_filenames(pDataObj, |filename| {
                drop_handler.send_event(Event::WindowEvent {
                    window_id: RootWindowId(WindowId(drop_handler.window)),
                    event: DroppedFile(filename),
                });
            })
        };
        if let Some(hdrop) = hdrop {
            unsafe { DragFinish(hdrop) };
        } else if let Some(names) = unsafe { virtual_file_names(pDataObj) } {
            // PdfCraft patch: files without a path on disk (Outlook attachments) are written to
            // a temporary folder and dropped from there.
            for (index, name) in names.iter().enumerate() {
                let saved = unsafe { virtual_file_contents(pDataObj, index) }
                    .and_then(|bytes| save_virtual_file(name, &bytes));
                match saved {
                    Some(path) => drop_handler.send_event(Event::WindowEvent {
                        window_id: RootWindowId(WindowId(drop_handler.window)),
                        event: DroppedFile(path),
                    }),
                    None => debug!("Could not read the dropped file {name:?}."),
                }
            }
        }

        S_OK
    }

    unsafe fn from_interface<'a, InterfaceT>(this: *mut InterfaceT) -> &'a mut FileDropHandlerData {
        unsafe { &mut *(this as *mut _) }
    }

    unsafe fn iterate_filenames<F>(data_obj: *const IDataObject, callback: F) -> Option<HDROP>
    where
        F: Fn(PathBuf),
    {
        let drop_format = FORMATETC {
            cfFormat: CF_HDROP,
            ptd: ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL as u32,
        };

        let mut medium = unsafe { std::mem::zeroed() };
        let get_data_fn = unsafe { (*(*data_obj).cast::<IDataObjectVtbl>()).GetData };
        let get_data_result = unsafe { get_data_fn(data_obj as *mut _, &drop_format, &mut medium) };
        if get_data_result >= 0 {
            let hdrop = unsafe { medium.u.hGlobal as HDROP };

            // The second parameter (0xFFFFFFFF) instructs the function to return the item count
            let item_count = unsafe { DragQueryFileW(hdrop, 0xffffffff, ptr::null_mut(), 0) };

            for i in 0..item_count {
                // Get the length of the path string NOT including the terminating null character.
                // Previously, this was using a fixed size array of MAX_PATH length, but the
                // Windows API allows longer paths under certain circumstances.
                let character_count =
                    unsafe { DragQueryFileW(hdrop, i, ptr::null_mut(), 0) as usize };
                let str_len = character_count + 1;

                // Fill path_buf with the null-terminated file name
                let mut path_buf = Vec::with_capacity(str_len);
                unsafe {
                    DragQueryFileW(hdrop, i, path_buf.as_mut_ptr(), str_len as u32);
                    path_buf.set_len(str_len);
                }

                callback(OsString::from_wide(&path_buf[0..character_count]).into());
            }

            Some(hdrop)
        } else if get_data_result == DV_E_FORMATETC {
            // If the dropped item is not a file this error will occur.
            // In this case it is OK to return without taking further action.
            debug!("Error occurred while processing dropped/hovered item: item is not a file.");
            None
        } else {
            debug!("Unexpected error occurred while processing dropped/hovered item.");
            None
        }
    }
}

// PdfCraft patch: "virtual files", dragged from apps that hold them only in memory, such as
// Outlook attachments or files inside a zip folder. `FileGroupDescriptorW` (or the ANSI
// `FileGroupDescriptor`) names them and `FileContents` holds each one's bytes.

/// The most virtual files read from one drop, and the most bytes read for one of them.
const MAX_VIRTUAL_FILES: usize = 256;
const MAX_VIRTUAL_FILE_BYTES: usize = 1 << 30;

fn clipboard_format(name: &str) -> Option<u16> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    u16::try_from(unsafe { RegisterClipboardFormatW(wide.as_ptr()) }).ok().filter(|f| *f != 0)
}

unsafe fn get_data(data_obj: *const IDataObject, format: &FORMATETC) -> Option<STGMEDIUM> {
    let mut medium: STGMEDIUM = unsafe { std::mem::zeroed() };
    let get_data_fn = unsafe { (*(*data_obj).cast::<IDataObjectVtbl>()).GetData };
    (unsafe { get_data_fn(data_obj as *mut _, format, &mut medium) } >= 0).then_some(medium)
}

/// The names of the virtual files in `data_obj`, if it holds any.
unsafe fn virtual_file_names(data_obj: *const IDataObject) -> Option<Vec<String>> {
    for wide in [true, false] {
        let Some(cf) = clipboard_format(if wide { "FileGroupDescriptorW" } else { "FileGroupDescriptor" })
        else {
            continue;
        };
        let format = FORMATETC {
            cfFormat: cf,
            ptd: ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL as u32,
        };
        let Some(mut medium) = (unsafe { get_data(data_obj, &format) }) else { continue };
        let names = if medium.tymed == TYMED_HGLOBAL as u32 {
            unsafe { read_descriptor(medium.u.hGlobal, wide) }
        } else {
            None
        };
        unsafe { ReleaseStgMedium(&mut medium) };
        if let Some(names) = names.filter(|n| !n.is_empty()) {
            return Some(names);
        }
    }
    None
}

/// The file names in a `FILEGROUPDESCRIPTORW` (or `A`), bounded by the memory's real size.
unsafe fn read_descriptor(memory: HGLOBAL, wide: bool) -> Option<Vec<String>> {
    let size = unsafe { GlobalSize(memory) };
    let base = (unsafe { GlobalLock(memory) }) as *const u8;
    if base.is_null() {
        return None;
    }
    let (first, entry) = if wide {
        (std::mem::offset_of!(FILEGROUPDESCRIPTORW, fgd), std::mem::size_of::<FILEDESCRIPTORW>())
    } else {
        (std::mem::offset_of!(FILEGROUPDESCRIPTORA, fgd), std::mem::size_of::<FILEDESCRIPTORA>())
    };
    let count = if size >= 4 { (unsafe { ptr::read_unaligned(base as *const u32) }) as usize } else { 0 };
    let fits = size.saturating_sub(first) / entry;
    let mut names = Vec::new();
    for i in 0..count.min(fits).min(MAX_VIRTUAL_FILES) {
        let at = unsafe { base.add(first + i * entry) };
        let name = if wide {
            let d = unsafe { ptr::read_unaligned(at as *const FILEDESCRIPTORW) };
            let file_name = d.cFileName;
            let len = file_name.iter().position(|c| *c == 0).unwrap_or(file_name.len());
            String::from_utf16_lossy(&file_name[..len])
        } else {
            let d = unsafe { ptr::read_unaligned(at as *const FILEDESCRIPTORA) };
            let file_name = d.cFileName;
            let len = file_name.iter().position(|c| *c == 0).unwrap_or(file_name.len());
            ansi_to_string(&file_name[..len])
        };
        names.push(name);
    }
    unsafe { GlobalUnlock(memory) };
    Some(names)
}

/// Text in the system's ANSI code page.
fn ansi_to_string(bytes: &[u8]) -> String {
    let Ok(len) = i32::try_from(bytes.len()) else { return String::new() };
    if len == 0 {
        return String::new();
    }
    let mut wide = vec![0u16; bytes.len()];
    let written =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), len) };
    wide.truncate(usize::try_from(written).unwrap_or(0));
    String::from_utf16_lossy(&wide)
}

/// The bytes of virtual file `index` in `data_obj`.
unsafe fn virtual_file_contents(data_obj: *const IDataObject, index: usize) -> Option<Vec<u8>> {
    let format = FORMATETC {
        cfFormat: clipboard_format("FileContents")?,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: i32::try_from(index).ok()?,
        tymed: (TYMED_ISTREAM | TYMED_HGLOBAL) as u32,
    };
    let mut medium = unsafe { get_data(data_obj, &format) }?;
    let bytes = if medium.tymed == TYMED_HGLOBAL as u32 {
        let memory = unsafe { medium.u.hGlobal };
        let size = unsafe { GlobalSize(memory) };
        let base = (unsafe { GlobalLock(memory) }) as *const u8;
        if base.is_null() || size > MAX_VIRTUAL_FILE_BYTES {
            None
        } else {
            let bytes = unsafe { std::slice::from_raw_parts(base, size) }.to_vec();
            unsafe { GlobalUnlock(memory) };
            Some(bytes)
        }
    } else if medium.tymed == TYMED_ISTREAM as u32 {
        unsafe { read_stream(medium.u.pstm) }
    } else {
        None
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    bytes
}

#[allow(non_snake_case)]
#[repr(C)]
struct ISequentialStreamVtbl {
    parent: IUnknownVtbl,
    Read: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> HRESULT,
}

/// Read a stream to its end.
unsafe fn read_stream(stream: IStream) -> Option<Vec<u8>> {
    if stream.is_null() {
        return None;
    }
    let read = unsafe { (**(stream as *mut *const ISequentialStreamVtbl)).Read };
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let mut n = 0u32;
        let hr = unsafe { read(stream, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n) };
        if hr < 0 {
            return None;
        }
        let n = (n as usize).min(buf.len());
        if n == 0 {
            return Some(out);
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > MAX_VIRTUAL_FILE_BYTES {
            return None;
        }
    }
}

/// Write a virtual file to a new temporary folder under its own name (without any folders
/// or characters Windows doesn't allow in a name).
fn save_virtual_file(name: &str, bytes: &[u8]) -> Option<PathBuf> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let base = name.rsplit(['\\', '/']).next().unwrap_or_default();
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let file = if clean.is_empty() { "attachment".to_string() } else { clean };
    let dir = std::env::temp_dir().join("dropped-files").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(file);
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

impl FileDropHandlerData {
    fn send_event(&self, event: Event<()>) {
        (self.send_event)(event);
    }
}

impl Drop for FileDropHandler {
    fn drop(&mut self) {
        unsafe {
            FileDropHandler::Release(self.data as *mut IUnknown);
        }
    }
}

static DROP_TARGET_VTBL: IDropTargetVtbl = IDropTargetVtbl {
    parent: IUnknownVtbl {
        QueryInterface: FileDropHandler::QueryInterface,
        AddRef: FileDropHandler::AddRef,
        Release: FileDropHandler::Release,
    },
    DragEnter: FileDropHandler::DragEnter,
    DragOver: FileDropHandler::DragOver,
    DragLeave: FileDropHandler::DragLeave,
    Drop: FileDropHandler::Drop,
};
