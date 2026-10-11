//! Macos screen have a y axis that goings up from the bottom of the screen and
//! an origin at the bottom left of the main display.
mod dispatcher;
mod display;
mod display_link;
mod events;
mod keyboard;
mod pasteboard;

#[cfg(feature = "screen-capture")]
mod screen_capture;

#[cfg(not(feature = "macos-blade"))]
mod metal_atlas;
#[cfg(not(feature = "macos-blade"))]
pub mod metal_renderer;

use core_video::image_buffer::CVImageBuffer;
#[cfg(not(feature = "macos-blade"))]
use metal_renderer as renderer;

#[cfg(feature = "macos-blade")]
use crate::platform::blade as renderer;

#[cfg(feature = "font-kit")]
mod open_type;

#[cfg(feature = "font-kit")]
mod text_system;

mod platform;
mod window;
mod window_appearance;

use crate::{DevicePixels, Pixels, Size, px, size};
use cocoa::{
    base::{id, nil},
    foundation::{NSAutoreleasePool, NSNotFound, NSRect, NSSize, NSString, NSUInteger},
};

use objc::runtime::{BOOL, NO, YES};
use std::{
    ffi::{CStr, c_char},
    ops::Range,
};

pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use display_link::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use window::*;

#[cfg(feature = "font-kit")]
pub(crate) use text_system::*;

/// A frame of video captured from a screen.
pub(crate) type PlatformScreenCaptureFrame = CVImageBuffer;

trait BoolExt {
    fn to_objc(self) -> BOOL;
}

impl BoolExt for bool {
    fn to_objc(self) -> BOOL {
        if self { YES } else { NO }
    }
}

trait NSStringExt {
    unsafe fn to_str(&self) -> &str;
}

impl NSStringExt for id {
    unsafe fn to_str(&self) -> &str {
        unsafe {
            let cstr = self.UTF8String();
            if cstr.is_null() {
                ""
            } else {
                CStr::from_ptr(cstr as *mut c_char).to_str().unwrap()
            }
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct NSRange {
    pub location: NSUInteger,
    pub length: NSUInteger,
}

impl NSRange {
    fn invalid() -> Self {
        Self {
            location: NSNotFound as NSUInteger,
            length: 0,
        }
    }

    fn is_valid(&self) -> bool {
        self.location != NSNotFound as NSUInteger
    }

    fn to_range(self) -> Option<Range<usize>> {
        if self.is_valid() {
            let start = self.location as usize;
            let end = start.checked_add(self.length as usize)?;
            Some(start..end)
        } else {
            None
        }
    }
}

impl From<Range<usize>> for NSRange {
    fn from(range: Range<usize>) -> Self {
        NSRange {
            location: range.start as NSUInteger,
            length: range.len() as NSUInteger,
        }
    }
}

unsafe impl objc::Encode for NSRange {
    fn encode() -> objc::Encoding {
        let encoding = format!(
            "{{NSRange={}{}}}",
            NSUInteger::encode().as_str(),
            NSUInteger::encode().as_str()
        );
        unsafe { objc::Encoding::from_str(&encoding) }
    }
}

/// Allow NSString::alloc use here because it sets autorelease
#[allow(clippy::disallowed_methods)]
unsafe fn ns_string(string: &str) -> id {
    unsafe { NSString::alloc(nil).init_str(string).autorelease() }
}

impl From<NSSize> for Size<Pixels> {
    fn from(value: NSSize) -> Self {
        Size {
            width: px(value.width as f32),
            height: px(value.height as f32),
        }
    }
}

impl From<NSRect> for Size<Pixels> {
    fn from(rect: NSRect) -> Self {
        let NSSize { width, height } = rect.size;
        size(width.into(), height.into())
    }
}

impl From<NSRect> for Size<DevicePixels> {
    fn from(rect: NSRect) -> Self {
        let NSSize { width, height } = rect.size;
        size(DevicePixels(width as i32), DevicePixels(height as i32))
    }
}

#[cfg(test)]
mod range_tests {
    use super::NSRange;

    #[test]
    fn native_ranges_preserve_representable_bounds() {
        for (location, length, expected) in [
            (0, 0, 0..0), (2, 5, 2..7),
            (1, usize::MAX - 1, 1..usize::MAX),
            (usize::MAX - 2, 2, usize::MAX - 2..usize::MAX),
        ] {
            assert_eq!(NSRange { location: location as _, length: length as _ }.to_range(), Some(expected));
        }
    }

    #[test]
    fn native_ranges_refuse_unavailable_and_overflow() {
        assert_eq!(NSRange::invalid().to_range(), None);
        for (location, length) in [(usize::MAX - 2, 3), (1, usize::MAX)] {
            assert_eq!(NSRange { location: location as _, length: length as _ }.to_range(), None);
        }
    }

    #[test]
    fn reverse_conversion_does_not_underflow() {
        let range = NSRange::from(std::ops::Range { start: 5, end: 2 });
        assert_eq!(range.location, 5);
        assert_eq!(range.length, 0);
        let range = NSRange::from(1..usize::MAX);
        assert_eq!(range.location, 1);
        assert_eq!(range.length as usize, usize::MAX - 1);
    }
}
