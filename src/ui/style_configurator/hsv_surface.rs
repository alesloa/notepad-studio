// SPDX-License-Identifier: Apache-2.0
//
// Reusable HSV surface + hue strip. Originally Style-Configurator-only;
// now lives behind the `HsvHost` trait so any dialog that owns the small
// set of HSV picker fields can host the surface (Preferences ▸ Dark Mode
// tones is the second consumer).
//
//   ┌────────────────┐ ┌──┐
//   │                │ │  │
//   │   SV surface   │ │  │  ← Hue strip (vertical, 6-segment gradient)
//   │  (sat × value) │ │  │     drag thumb sets the host's hue
//   │                │ │  │
//   │   ⊙ thumb      │ │⊙ │
//   └────────────────┘ └──┘
//
// SV surface paints two stacked quads via `linear_gradient`:
//   1. horizontal: white → pure-hue colour at host hue
//   2. vertical:   transparent → black
// Layering those gives the classic HSV saturation × value rectangle. A
// small thumb circle is painted at (sat × width, (1−val) × height).
//
// Hue strip paints 6 stacked quads (one per primary→primary segment) so
// the full rainbow shows; a horizontal tick marks `hue * height`.
//
// State lives on the host (whichever dialog implements `HsvHost`):
//   - hue / sat / val — current HSV
//   - drag — None / SvSurface / HueStrip while a drag is live
//   - hex buffer — the source-of-truth string
//
// The hex buffer is the source of truth; every drag rewrites the buffer
// via `set_buffer_from_hsv`, which calls the host's
// `on_hsv_picker_buffer_changed` so the consumer's live-preview /
// notification path stays in one place. `sync_from_buffer` runs after a
// non-HSV buffer change (typed hex, swatch click, picker open) so the
// SV / hue thumbs land on the right spot. It preserves the existing hue
// when the new colour is grey (s ≈ 0) — without that, dragging through
// the grey midline would snap hue to 0 and the user would lose their
// place on the hue strip.

use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, Entity, Hsla, IntoElement, MouseButton,
    ParentElement, Pixels, Rgba, SharedString, Styled, Window, div, fill, linear_color_stop,
    linear_gradient, point, prelude::*, px, rems, rgb, size,
};

use crate::style_model::style::{format_hex_rgb, parse_hex_rgb};
use crate::theme::Theme;

const SV_WIDTH_PX: f32 = 168.0;
const SV_HEIGHT_PX: f32 = 108.0;
const HUE_WIDTH_PX: f32 = 18.0;
const HUE_HEIGHT_PX: f32 = 108.0;
const THUMB_RADIUS_PX: f32 = 5.0;
const HUE_TICK_HALF_PX: f32 = 2.0;

/// Which HSV control (if any) the user is currently dragging. The
/// window-global `MouseMoveEvent` handlers consult this so only the
/// dragged channel updates on each move event — without it, moving the
/// cursor over the SV surface while a hue drag was active would also
/// move SV.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum HsvDrag {
    None,
    SvSurface,
    HueStrip,
}

/// Implemented by dialogs that own the HSV picker state and want to
/// embed the surface. Slim by design — every method is HSV-picker
/// plumbing, nothing more. Visibility is `pub(crate)` so consumers
/// outside `style_configurator/` can implement it.
pub(crate) trait HsvHost: 'static + Sized {
    fn picker_hue(&self) -> f32;
    fn picker_sat(&self) -> f32;
    fn picker_val(&self) -> f32;
    fn picker_hsv_drag(&self) -> HsvDrag;

    fn set_picker_hue(&mut self, h: f32);
    fn set_picker_sat(&mut self, s: f32);
    fn set_picker_val(&mut self, v: f32);
    fn set_picker_hsv_drag(&mut self, drag: HsvDrag);

    /// Read the source-of-truth hex buffer text (e.g. `"FF00AA"`,
    /// possibly empty during fresh picker open).
    fn picker_hex_text(&self) -> &str;

    /// Replace the hex buffer and reset the host's cursor/anchor to
    /// the end of the new text. Host-specific because cursor/anchor
    /// field names differ (Style Configurator has its own buffer,
    /// Preferences routes through `picker_hex.text`).
    fn set_picker_hex_text(&mut self, text: String);

    /// Called after `set_buffer_from_hsv` mutates the buffer. The host
    /// runs whatever live-preview / notify path it needs. Style
    /// Configurator pushes the new hex into the active style preview;
    /// Preferences just calls `cx.notify()` because the swatch popover
    /// has no live preview surface beyond the hex input.
    fn on_hsv_picker_buffer_changed(&mut self, cx: &mut Context<Self>);
}

/// HSV → packed 0xRRGGBB. `h` wraps via `fract`; `s` and `v` clamp to
/// `[0, 1]`. Standard formula, matches the picker-grid's `hsl_to_rgb`
/// shape so the maths feels familiar to a future maintainer.
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> u32 {
    let h = ((h % 1.0) + 1.0) % 1.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let h6 = h * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let r = ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    let g = ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    let b = ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (r << 16) | (g << 8) | b
}

/// RGB → HSV. `(h, s, v)` with `h` in `[0, 1)`, `s`/`v` in `[0, 1]`. For
/// pure greys (max == min) hue is returned as `0.0`; callers preserve
/// the prior hue when `s == 0` so dragging brightness through grey
/// doesn't snap the hue thumb to red.
pub(crate) fn rgb_to_hsv(rgb: u32) -> (f32, f32, f32) {
    let r = ((rgb >> 16) & 0xFF) as f32 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f32 / 255.0;
    let b = (rgb & 0xFF) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if max <= 0.0 { 0.0 } else { delta / max };
    let h = if delta <= 0.0 {
        0.0
    } else if (max - r).abs() < f32::EPSILON {
        (((g - b) / delta) % 6.0 + 6.0) % 6.0 / 6.0
    } else if (max - g).abs() < f32::EPSILON {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    (h, s, v)
}

/// Re-decompose the host's hex buffer into HSV after a non-HSV mutation
/// (hex typing, swatch click, picker open). Keeps the existing hue
/// when the new colour is grey — otherwise dragging brightness down
/// to black would zero the hue and the user would lose their place on
/// the hue strip. No-op when the buffer doesn't parse as 6-char hex.
pub(crate) fn sync_from_buffer<H: HsvHost>(host: &mut H) {
    let Some(rgb) = parse_hex_rgb(host.picker_hex_text()) else {
        return;
    };
    let (h, s, v) = rgb_to_hsv(rgb);
    if s > 0.0 {
        host.set_picker_hue(h);
    }
    host.set_picker_sat(s);
    host.set_picker_val(v);
}

/// Push the host's current HSV onto its hex buffer and refresh the
/// live preview through the host's standard path. Called from the
/// SV-surface and hue-strip drag handlers.
pub(crate) fn set_buffer_from_hsv<H: HsvHost>(host: &mut H, cx: &mut Context<H>) {
    let rgb = hsv_to_rgb(host.picker_hue(), host.picker_sat(), host.picker_val());
    let buf = format_hex_rgb(rgb);
    host.set_picker_hex_text(buf);
    host.on_hsv_picker_buffer_changed(cx);
}

/// SV-surface + hue-strip row. `id_prefix` namespaces the two box IDs
/// so multiple consumers can mount the surface in the same render tree
/// without GPUI element-ID collisions (every consumer passes its own
/// prefix, e.g. `"style-cfg"`, `"prefs"`).
pub(crate) fn render<H: HsvHost>(
    theme: &Theme,
    id_prefix: &'static str,
    entity: &Entity<H>,
) -> AnyElement {
    let sv_id: SharedString = format!("{id_prefix}-sv-surface-box").into();
    let hue_id: SharedString = format!("{id_prefix}-hue-strip-box").into();
    div()
        .flex()
        .flex_row()
        .gap(rems(8. / 16.))
        .child(
            div()
                .id(gpui::ElementId::Name(sv_id))
                .w(px(SV_WIDTH_PX))
                .h(px(SV_HEIGHT_PX))
                .border_1()
                .border_color(rgb(theme.group_box_border))
                .cursor(CursorStyle::PointingHand)
                .child(SvSurface {
                    dialog: entity.clone(),
                }),
        )
        .child(
            div()
                .id(gpui::ElementId::Name(hue_id))
                .w(px(HUE_WIDTH_PX))
                .h(px(HUE_HEIGHT_PX))
                .border_1()
                .border_color(rgb(theme.group_box_border))
                .cursor(CursorStyle::PointingHand)
                .child(HueStrip {
                    dialog: entity.clone(),
                }),
        )
        .into_any_element()
}

// --- SV surface element ---------------------------------------------------

pub(crate) struct SvSurface<H: HsvHost> {
    pub(crate) dialog: Entity<H>,
}

impl<H: HsvHost> IntoElement for SvSurface<H> {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl<H: HsvHost> gpui::Element for SvSurface<H> {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_ref = self.dialog.read(cx);
        let hue = dialog_ref.picker_hue();
        let sat = dialog_ref.picker_sat();
        let val = dialog_ref.picker_val();
        let pure_hue_rgb = hsv_to_rgb(hue, 1.0, 1.0);

        let pure_hue: Hsla = Rgba::from(rgb(pure_hue_rgb)).into();
        let white: Hsla = Rgba::from(rgb(0xFFFFFF)).into();
        let black: Hsla = Rgba::from(rgb(0x000000)).into();
        let transparent: Hsla = Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        };

        // Saturation gradient — white at sat=0 (left) to pure-hue at sat=1
        // (right). `linear_gradient` angle 90° means left → right per
        // gpui's MDN-aligned convention.
        window.paint_quad(fill(
            bounds,
            linear_gradient(
                90.0,
                linear_color_stop(white, 0.0),
                linear_color_stop(pure_hue, 1.0),
            ),
        ));

        // Value gradient — transparent at top (val=1) to opaque black at
        // bottom (val=0). Overlays the saturation gradient so the
        // bottom edge fades to black regardless of hue.
        window.paint_quad(fill(
            bounds,
            linear_gradient(
                180.0,
                linear_color_stop(transparent, 0.0),
                linear_color_stop(black, 1.0),
            ),
        ));

        // Thumb circle at (sat × width, (1 − val) × height). Outer dark
        // ring + inner white ring keeps it visible against any colour.
        let cx_pos = bounds.left() + bounds.size.width * sat;
        let cy_pos = bounds.top() + bounds.size.height * (1.0 - val);
        paint_thumb_ring(window, cx_pos, cy_pos);

        // Mouse-down inside the surface → start a drag, snap thumb to
        // click position. Window-global so the drag tracks past the
        // surface edges (matching every other text input + the hex
        // field's drag-select).
        let dialog_for_down = self.dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            cx.stop_propagation();
            let local_x = f32::from(event.position.x - bounds.left());
            let local_y = f32::from(event.position.y - bounds.top());
            let new_sat = (local_x / f32::from(bounds.size.width)).clamp(0.0, 1.0);
            let new_val = 1.0 - (local_y / f32::from(bounds.size.height)).clamp(0.0, 1.0);
            dialog_for_down.update(cx, |this, cx| {
                this.set_picker_hsv_drag(HsvDrag::SvSurface);
                this.set_picker_sat(new_sat);
                this.set_picker_val(new_val);
                set_buffer_from_hsv(this, cx);
            });
        });

        // Mouse-move during SV drag — extend the drag, even after the
        // cursor leaves the surface.
        let dialog_for_move = self.dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || !event.dragging() {
                return;
            }
            let drag = dialog_for_move.read(cx).picker_hsv_drag();
            if drag != HsvDrag::SvSurface {
                return;
            }
            let local_x = f32::from(event.position.x - bounds.left());
            let local_y = f32::from(event.position.y - bounds.top());
            let new_sat = (local_x / f32::from(bounds.size.width)).clamp(0.0, 1.0);
            let new_val = 1.0 - (local_y / f32::from(bounds.size.height)).clamp(0.0, 1.0);
            dialog_for_move.update(cx, |this, cx| {
                if (this.picker_sat() - new_sat).abs() > 1e-4
                    || (this.picker_val() - new_val).abs() > 1e-4
                {
                    this.set_picker_sat(new_sat);
                    this.set_picker_val(new_val);
                    set_buffer_from_hsv(this, cx);
                }
            });
        });

        // Mouse-up — end an SV drag regardless of where the release
        // happens.
        let dialog_for_up = self.dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            dialog_for_up.update(cx, |this, _cx| {
                if this.picker_hsv_drag() == HsvDrag::SvSurface {
                    this.set_picker_hsv_drag(HsvDrag::None);
                }
            });
        });
    }
}

// --- Hue strip element ----------------------------------------------------

pub(crate) struct HueStrip<H: HsvHost> {
    pub(crate) dialog: Entity<H>,
}

impl<H: HsvHost> IntoElement for HueStrip<H> {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl<H: HsvHost> gpui::Element for HueStrip<H> {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = gpui::Style::default();
        style.size.width = gpui::relative(1.).into();
        style.size.height = gpui::relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&gpui::GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_ref = self.dialog.read(cx);
        let hue = dialog_ref.picker_hue();

        // Six vertical segments. Each segment is a 2-stop gradient
        // between adjacent primaries (R → Y → G → C → B → M → R) so the
        // full rainbow paints without needing GPUI to grow multi-stop
        // gradients.
        const STOPS: [u32; 7] = [
            0xFF0000, // red
            0xFFFF00, // yellow
            0x00FF00, // green
            0x00FFFF, // cyan
            0x0000FF, // blue
            0xFF00FF, // magenta
            0xFF0000, // red (wrap)
        ];
        let seg_h = f32::from(bounds.size.height) / 6.0;
        for i in 0..6 {
            let top: Hsla = Rgba::from(rgb(STOPS[i])).into();
            let bot: Hsla = Rgba::from(rgb(STOPS[i + 1])).into();
            let seg = Bounds::new(
                point(bounds.left(), bounds.top() + px(seg_h * i as f32)),
                size(bounds.size.width, px(seg_h)),
            );
            window.paint_quad(fill(
                seg,
                linear_gradient(
                    180.0,
                    linear_color_stop(top, 0.0),
                    linear_color_stop(bot, 1.0),
                ),
            ));
        }

        // Tick mark across the strip at the current hue. Two thin black
        // rectangles top + bottom of the tick row keep it visible
        // against any segment colour.
        let tick_y = bounds.top() + bounds.size.height * hue;
        let tick_top = (tick_y - px(HUE_TICK_HALF_PX)).max(bounds.top());
        let tick_h = px(HUE_TICK_HALF_PX * 2.0).min(bounds.bottom() - tick_top);
        let outline_color: Hsla = Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 1.0,
        };
        let inline_color: Hsla = Hsla {
            h: 0.0,
            s: 0.0,
            l: 1.0,
            a: 1.0,
        };
        window.paint_quad(fill(
            Bounds::new(point(bounds.left(), tick_top), size(bounds.size.width, tick_h)),
            outline_color,
        ));
        let inner_top = tick_top + px(1.0);
        let inner_h = (tick_h - px(2.0)).max(px(0.0));
        if inner_h > px(0.0) {
            window.paint_quad(fill(
                Bounds::new(
                    point(bounds.left() + px(1.0), inner_top),
                    size((bounds.size.width - px(2.0)).max(px(0.0)), inner_h),
                ),
                inline_color,
            ));
        }

        let dialog_for_down = self.dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !bounds.contains(&event.position) {
                return;
            }
            cx.stop_propagation();
            let local_y = f32::from(event.position.y - bounds.top());
            let new_hue = (local_y / f32::from(bounds.size.height)).clamp(0.0, 0.9999);
            dialog_for_down.update(cx, |this, cx| {
                this.set_picker_hsv_drag(HsvDrag::HueStrip);
                this.set_picker_hue(new_hue);
                set_buffer_from_hsv(this, cx);
            });
        });

        let dialog_for_move = self.dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || !event.dragging() {
                return;
            }
            let drag = dialog_for_move.read(cx).picker_hsv_drag();
            if drag != HsvDrag::HueStrip {
                return;
            }
            let local_y = f32::from(event.position.y - bounds.top());
            let new_hue = (local_y / f32::from(bounds.size.height)).clamp(0.0, 0.9999);
            dialog_for_move.update(cx, |this, cx| {
                if (this.picker_hue() - new_hue).abs() > 1e-4 {
                    this.set_picker_hue(new_hue);
                    set_buffer_from_hsv(this, cx);
                }
            });
        });

        let dialog_for_up = self.dialog.clone();
        window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _w, cx| {
            if phase != gpui::DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            dialog_for_up.update(cx, |this, _cx| {
                if this.picker_hsv_drag() == HsvDrag::HueStrip {
                    this.set_picker_hsv_drag(HsvDrag::None);
                }
            });
        });
    }
}

/// Paint a small thumb ring at `(cx, cy)` — black outer rim + white
/// inner rim. Two stacked filled quads instead of a real circle (GPUI
/// has no native circle primitive); square thumb reads as a target
/// crosshair against the gradient.
fn paint_thumb_ring(window: &mut Window, cx_pos: Pixels, cy_pos: Pixels) {
    let outer_color: Hsla = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: 1.0,
    };
    let inner_color: Hsla = Hsla {
        h: 0.0,
        s: 0.0,
        l: 1.0,
        a: 1.0,
    };
    let r = px(THUMB_RADIUS_PX);
    let outer = Bounds::new(
        point(cx_pos - r, cy_pos - r),
        size(r * 2.0, r * 2.0),
    );
    window.paint_quad(fill(outer, outer_color));
    let inner_r = px(THUMB_RADIUS_PX - 1.5);
    let inner = Bounds::new(
        point(cx_pos - inner_r, cy_pos - inner_r),
        size(inner_r * 2.0, inner_r * 2.0),
    );
    window.paint_quad(fill(inner, inner_color));
    let center_r = px(THUMB_RADIUS_PX - 3.0);
    if center_r > px(0.0) {
        let center = Bounds::new(
            point(cx_pos - center_r, cy_pos - center_r),
            size(center_r * 2.0, center_r * 2.0),
        );
        window.paint_quad(fill(center, outer_color));
    }
}
