use super::{
    button_from_mouse_event, location_hash, modifiers_from_kb_event, modifiers_from_mouse_event,
    modifiers_from_wheel_event, pos_from_mouse_event, prefers_color_scheme_dark, primary_touch_pos,
    push_touches, text_from_keyboard_event, theme_from_dark_mode, translate_key, AppRunner,
    Closure, JsCast, JsValue, WebRunner,
};
use web_sys::EventTarget;

// TODO(emilk): there are more calls to `prevent_default` and `stop_propagaton`
// than what is probably needed.

// ------------------------------------------------------------------------

/// Calls `request_animation_frame` to schedule repaint.
///
/// It will only paint if needed, but will always call `request_animation_frame` immediately.
pub(crate) fn paint_and_schedule(runner_ref: &WebRunner) -> Result<(), JsValue> {
    // Only paint and schedule if there has been no panic
    if let Some(mut runner_lock) = runner_ref.try_lock() {
        paint_if_needed(&mut runner_lock);
        drop(runner_lock);
        runner_ref.request_animation_frame()?;
    }
    Ok(())
}

fn paint_if_needed(runner: &mut AppRunner) {
    if runner.needs_repaint.needs_repaint() {
        if runner.has_outstanding_paint_data() {
            // We have already run the logic, e.g. in an on-click event,
            // so let's only present the results:
            runner.paint();

            // We schedule another repaint asap, so that we can run the actual logic
            // again, which may schedule a new repaint (if there's animations):
            runner.needs_repaint.repaint_asap();
        } else {
            // Clear the `needs_repaint` flags _before_
            // running the logic, as the logic could cause it to be set again.
            runner.needs_repaint.clear();

            let mut stopwatch = crate::stopwatch::Stopwatch::new();
            stopwatch.start();

            // Run user code…
            runner.logic();

            // …and paint the result.
            runner.paint();

            runner.report_frame_time(stopwatch.total_time_sec());
        }
    }
    runner.auto_save_if_needed();
}

// ------------------------------------------------------------------------

pub(crate) fn install_event_handlers(runner_ref: &WebRunner) -> Result<(), JsValue> {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let canvas = runner_ref.try_lock().unwrap().canvas().clone();

    // MEMBRANE: for some reason canvas doesn't get these events in the vscode iframe so use window instead.
    install_blur_focus(runner_ref, &window)?;

    prevent_default_and_stop_propagation(
        runner_ref,
        &canvas,
        &[
            // Allow users to use ctrl-p for e.g. a command palette:
            "afterprint",
            // By default, right-clicks open a browser context menu.
            // We don't want to do that (right clicks are handled by egui):
            "contextmenu",
        ],
    )?;

    install_keydown(runner_ref, &canvas)?;
    install_keyup(runner_ref, &canvas)?;

    // It seems copy/cut/paste events only work on the document,
    // so we check if we have focus inside of the handler.
    install_copy_cut_paste(runner_ref, &document)?;

    // Use `document` here to notice if the user releases a drag outside of the canvas:
    // See https://github.com/emilk/egui/issues/3157
    install_mousemove(runner_ref, &document)?;
    install_pointerup(runner_ref, &document)?;
    install_pointerdown(runner_ref, &canvas)?;
    install_mouseleave(runner_ref, &canvas)?;

    install_touchstart(runner_ref, &canvas)?;
    // Use `document` here to notice if the user drag outside of the canvas:
    // See https://github.com/emilk/egui/issues/3157
    install_touchmove(runner_ref, &document)?;
    install_touchend(runner_ref, &document)?;
    install_touchcancel(runner_ref, &canvas)?;

    install_wheel(runner_ref, &canvas)?;
    install_drag_and_drop(runner_ref, &canvas)?;
    install_window_events(runner_ref, &window)?;
    install_color_scheme_change_event(runner_ref, &window)?;
    Ok(())
}

fn install_blur_focus(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    // NOTE: because of the text agent we sometime miss 'blur' events,
    // so we also poll the focus state each frame in `AppRunner::logic`.
    for event_name in ["blur", "focus"] {
        let closure = move |_event: web_sys::MouseEvent, runner: &mut AppRunner| {
            log::trace!("{} {event_name:?}", runner.canvas().id());
            runner.update_focus();

            if event_name == "blur" {
                // This might be a good time to save the state
                runner.save();

                // MEMBRANE: Some keyboard shortcuts steal away the focus from egui which means we won't get keyup
                // events for those shortcut keys.
                let keys_down = runner.egui_ctx().input(|i| i.keys_down.clone());
                for key in keys_down {
                    runner.input.raw.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        };

        runner_ref.add_event_listener(target, event_name, closure)?;
    }
    Ok(())
}

fn install_keydown(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(
        target,
        "keydown",
        |event: web_sys::KeyboardEvent, runner| {
            if !runner.input.raw.focused {
                return;
            }

            let modifiers = modifiers_from_kb_event(&event);
            if !modifiers.ctrl
                && !modifiers.command
                // When text agent is focused, it is responsible for handling input events
                && !runner.text_agent.has_focus()
            {
                if let Some(text) = text_from_keyboard_event(&event) {
                    let egui_event = egui::Event::Text(text);
                    let should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
                    runner.input.raw.events.push(egui_event);
                    runner.needs_repaint.repaint_asap();

                    // If this is indeed text, then prevent any other action.
                    event.prevent_default();

                    // Use web options to tell if the event should be propagated to parent elements.
                    if !should_propagate {
                        event.stop_propagation();
                    }
                }
            }

            on_keydown(event, runner);
        },
    )
}

#[allow(clippy::needless_pass_by_value)] // So that we can pass it directly to `add_event_listener`
pub(crate) fn on_keydown(event: web_sys::KeyboardEvent, runner: &mut AppRunner) {
    let has_focus = runner.input.raw.focused;
    if !has_focus {
        return;
    }

    if event.is_composing() || event.key_code() == 229 {
        // https://web.archive.org/web/20200526195704/https://www.fxsitecompat.dev/en-CA/docs/2018/keydown-and-keyup-events-are-now-fired-during-ime-composition/
        return;
    }

    let modifiers = modifiers_from_kb_event(&event);
    runner.input.raw.modifiers = modifiers;

    let key = event.key();
    let egui_key = translate_key(&key);

    if let Some(egui_key) = egui_key {
        let egui_event = egui::Event::Key {
            key: egui_key,
            physical_key: None, // TODO(fornwall)
            pressed: true,
            repeat: false, // egui will fill this in for us!
            modifiers,
        };
        let _should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
        runner.input.raw.events.push(egui_event);
        runner.needs_repaint.repaint_asap();

        let prevent_default = should_prevent_default_for_key(runner, &modifiers, egui_key);

        // log::debug!(
        //     "On keydown {:?} {egui_key:?}, has_focus: {has_focus}, egui_wants_keyboard: {}, prevent_default: {prevent_default}",
        //     event.key().as_str(),
        //     runner.egui_ctx().wants_keyboard_input()
        // );

        if prevent_default {
            event.prevent_default();
        }

        // MEMBRANE: Stopping propagation here prevents vscode from handling key events while gaze is focused.
        // TODO: use the newer should_propagate_event hook
        let should_propagate = true;

        // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
        if !should_propagate {
            event.stop_propagation();
        }
    }
}

/// If the canvas (or text agent) has focus:
/// should we prevent the default browser event action when the user presses this key?
fn should_prevent_default_for_key(
    runner: &AppRunner,
    modifiers: &egui::Modifiers,
    egui_key: egui::Key,
) -> bool {
    // NOTE: We never want to prevent:
    // * F5 / cmd-R (refresh)
    // * cmd-shift-C (debug tools)
    // * cmd/ctrl-c/v/x (lest we prevent copy/paste/cut events)

    // Prevent ctrl-P from opening the print dialog. Users may want to use it for a command palette.
    if egui_key == egui::Key::P && (modifiers.ctrl || modifiers.command || modifiers.mac_cmd) {
        return true;
    }

    if egui_key == egui::Key::Space && !runner.text_agent.has_focus() {
        // Space scrolls the web page, but we don't want that while canvas has focus
        // However, don't prevent it if text agent has focus, or we can't type space!
        return true;
    }

    matches!(
        egui_key,
        // Prevent browser from focusing the next HTML element.
        // egui uses Tab to move focus within the egui app.
        egui::Key::Tab

        // So we don't go back to previous page while canvas has focus
        | egui::Key::Backspace

        // Don't scroll web page while canvas has focus.
        // Also, cmd-left is "back" on Mac (https://github.com/emilk/egui/issues/58)
        | egui::Key::ArrowDown | egui::Key::ArrowLeft | egui::Key::ArrowRight |  egui::Key::ArrowUp
    )
}

fn install_keyup(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "keyup", on_keyup)
}

#[allow(clippy::needless_pass_by_value)] // So that we can pass it directly to `add_event_listener`
pub(crate) fn on_keyup(event: web_sys::KeyboardEvent, runner: &mut AppRunner) {
    let modifiers = modifiers_from_kb_event(&event);
    runner.input.raw.modifiers = modifiers;

    let mut _propagate_event = false;

    if let Some(key) = translate_key(&event.key()) {
        let egui_event = egui::Event::Key {
            key,
            physical_key: None, // TODO(fornwall)
            pressed: false,
            repeat: false,
            modifiers,
        };
        _propagate_event |= (runner.web_options.should_propagate_event)(&egui_event);
        runner.input.raw.events.push(egui_event);
    }

    if event.key() == "Meta" || event.key() == "Control" {
        // When pressing Cmd+A (select all) or Ctrl+C (copy),
        // chromium will not fire a `keyup` for the letter key.
        // This leads to stuck keys, unless we do this hack.
        // See https://github.com/emilk/egui/issues/4724

        let keys_down = runner.egui_ctx().input(|i| i.keys_down.clone());
        for key in keys_down {
            let egui_event = egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            };
            _propagate_event |= (runner.web_options.should_propagate_event)(&egui_event);
            runner.input.raw.events.push(egui_event);
        }
    }

    runner.needs_repaint.repaint_asap();

    // MEMBRANE: Stopping propagation here prevents vscode from handling key events while gaze is focused.
    // TODO: use the newer should_propagate_event hook
    let propagate_event = true;

    // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
    let has_focus = runner.input.raw.focused;
    if has_focus && !propagate_event {
        event.stop_propagation();
    }
}

fn install_copy_cut_paste(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "paste", |event: web_sys::ClipboardEvent, runner| {
        if let Some(data) = event.clipboard_data() {
            if let Ok(text) = data.get_data("text") {
                let text = text.replace("\r\n", "\n");

                let mut should_propagate = false;
                if !text.is_empty() && runner.input.raw.focused {
                    let egui_event = egui::Event::Paste(text);
                    should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
                    runner.input.raw.events.push(egui_event);
                    runner.needs_repaint.repaint_asap();
                }

                // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
                if !should_propagate {
                    event.stop_propagation();
                }
                event.prevent_default();
            }
        }
    })?;

    runner_ref.add_event_listener(target, "cut", |event: web_sys::ClipboardEvent, runner| {
        if runner.input.raw.focused {
            runner.input.raw.events.push(egui::Event::Cut);

            // In Safari we are only allowed to write to the clipboard during the
            // event callback, which is why we run the app logic here and now:
            runner.logic();

            // Make sure we paint the output of the above logic call asap:
            runner.needs_repaint.repaint_asap();
        }

        // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
        if !(runner.web_options.should_propagate_event)(&egui::Event::Cut) {
            event.stop_propagation();
        }
        event.prevent_default();
    })?;

    runner_ref.add_event_listener(target, "copy", |event: web_sys::ClipboardEvent, runner| {
        if runner.input.raw.focused {
            runner.input.raw.events.push(egui::Event::Copy);

            // In Safari we are only allowed to write to the clipboard during the
            // event callback, which is why we run the app logic here and now:
            runner.logic();

            // Make sure we paint the output of the above logic call asap:
            runner.needs_repaint.repaint_asap();
        }

        // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
        if !(runner.web_options.should_propagate_event)(&egui::Event::Copy) {
            event.stop_propagation();
        }
        event.prevent_default();
    })?;

    Ok(())
}

fn install_window_events(runner_ref: &WebRunner, window: &EventTarget) -> Result<(), JsValue> {
    // Save-on-close
    runner_ref.add_event_listener(window, "onbeforeunload", |_: web_sys::Event, runner| {
        runner.save();
    })?;

    // NOTE: resize is handled by `ResizeObserver` below
    for event_name in &["load", "pagehide", "pageshow"] {
        runner_ref.add_event_listener(window, event_name, move |_: web_sys::Event, runner| {
            // log::debug!("{event_name:?}");
            runner.needs_repaint.repaint_asap();
        })?;
    }

    runner_ref.add_event_listener(window, "hashchange", |_: web_sys::Event, runner| {
        // `epi::Frame::info(&self)` clones `epi::IntegrationInfo`, but we need to modify the original here
        runner.frame.info.web_info.location.hash = location_hash();
        runner.needs_repaint.repaint_asap(); // tell the user about the new hash
    })?;

    Ok(())
}

fn install_color_scheme_change_event(
    runner_ref: &WebRunner,
    window: &web_sys::Window,
) -> Result<(), JsValue> {
    if let Some(media_query_list) = prefers_color_scheme_dark(window)? {
        runner_ref.add_event_listener::<web_sys::MediaQueryListEvent>(
            &media_query_list,
            "change",
            |event, runner| {
                let theme = theme_from_dark_mode(event.matches());
                runner.input.raw.system_theme = Some(theme);
                runner.needs_repaint.repaint_asap();
            },
        )?;
    }

    Ok(())
}

fn prevent_default_and_stop_propagation(
    runner_ref: &WebRunner,
    target: &EventTarget,
    event_names: &[&'static str],
) -> Result<(), JsValue> {
    for event_name in event_names {
        let closure = move |event: web_sys::MouseEvent, _runner: &mut AppRunner| {
            event.prevent_default();
            event.stop_propagation();
            // log::debug!("Preventing event {event_name:?}");
        };

        runner_ref.add_event_listener(target, event_name, closure)?;
    }

    Ok(())
}

fn install_pointerdown(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(
        target,
        "pointerdown",
        |event: web_sys::PointerEvent, runner: &mut AppRunner| {
            let modifiers = modifiers_from_mouse_event(&event);
            runner.input.raw.modifiers = modifiers;
            let mut should_propagate = false;
            if let Some(button) = button_from_mouse_event(&event) {
                let pos = pos_from_mouse_event(runner.canvas(), &event, runner.egui_ctx());
                let modifiers = runner.input.raw.modifiers;
                let egui_event = egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers,
                };
                should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
                runner.input.raw.events.push(egui_event);

                // In Safari we are only allowed to write to the clipboard during the
                // event callback, which is why we run the app logic here and now:
                runner.logic();

                // Make sure we paint the output of the above logic call asap:
                runner.needs_repaint.repaint_asap();
            }

            // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
            if !should_propagate {
                event.stop_propagation();
            }
            // Note: prevent_default breaks VSCode tab focusing, hence why we don't call it here.
        },
    )
}

fn install_pointerup(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(
        target,
        "pointerup",
        |event: web_sys::PointerEvent, runner| {
            let modifiers = modifiers_from_mouse_event(&event);
            runner.input.raw.modifiers = modifiers;

            let pos = pos_from_mouse_event(runner.canvas(), &event, runner.egui_ctx());

            if is_interested_in_pointer_event(
                runner,
                egui::pos2(event.client_x() as f32, event.client_y() as f32),
            ) {
                if let Some(button) = button_from_mouse_event(&event) {
                    let modifiers = runner.input.raw.modifiers;
                    let egui_event = egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: false,
                        modifiers,
                    };
                    let should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
                    runner.input.raw.events.push(egui_event);

                    // Previously on iOS, the canvas would not receive focus on
                    // any touch event, which resulted in the on-screen keyboard
                    // not working when focusing on a text field in an egui app.
                    // This attempts to fix that by forcing the focus on any
                    // click on the canvas.
                    runner.canvas().focus().ok();

                    // In Safari we are only allowed to do certain things
                    // (like playing audio, start a download, etc)
                    // on user action, such as a click.
                    // So we need to run the app logic here and now:
                    runner.logic();

                    // Make sure we paint the output of the above logic call asap:
                    runner.needs_repaint.repaint_asap();

                    event.prevent_default();

                    // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
                    if !should_propagate {
                        event.stop_propagation();
                    }
                }
            }
        },
    )
}

/// Returns true if the cursor is above the canvas, or if we're dragging something.
/// Pass in the position in browser viewport coordinates (usually event.clientX/Y).
fn is_interested_in_pointer_event(runner: &AppRunner, pos: egui::Pos2) -> bool {
    let document = web_sys::window().unwrap().document().unwrap();
    let is_hovering_canvas = document
        .element_from_point(pos.x, pos.y)
        .is_some_and(|element| element.eq(runner.canvas()));
    let is_pointer_down = runner
        .egui_ctx()
        .input(|i| i.pointer.any_down() || i.any_touches());

    is_hovering_canvas || is_pointer_down
}

fn install_mousemove(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "mousemove", |event: web_sys::MouseEvent, runner| {
        let modifiers = modifiers_from_mouse_event(&event);
        runner.input.raw.modifiers = modifiers;

        let pos = pos_from_mouse_event(runner.canvas(), &event, runner.egui_ctx());

        if is_interested_in_pointer_event(
            runner,
            egui::pos2(event.client_x() as f32, event.client_y() as f32),
        ) {
            let egui_event = egui::Event::PointerMoved(pos);
            let should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
            runner.input.raw.events.push(egui_event);
            runner.needs_repaint.repaint_asap();

            // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
            if !should_propagate {
                event.stop_propagation();
            }
            event.prevent_default();
        }
    })
}

fn install_mouseleave(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(
        target,
        "mouseleave",
        |event: web_sys::MouseEvent, runner| {
            runner.input.raw.events.push(egui::Event::PointerGone);
            runner.needs_repaint.repaint_asap();

            // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
            if !(runner.web_options.should_propagate_event)(&egui::Event::PointerGone) {
                event.stop_propagation();
            }
            event.prevent_default();
        },
    )
}

fn install_touchstart(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(
        target,
        "touchstart",
        |event: web_sys::TouchEvent, runner| {
            let mut should_propagate = false;
            if let Some((pos, _)) = primary_touch_pos(runner, &event) {
                let egui_event = egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: runner.input.raw.modifiers,
                };
                should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
                runner.input.raw.events.push(egui_event);
            }

            push_touches(runner, egui::TouchPhase::Start, &event);
            runner.needs_repaint.repaint_asap();

            // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
            if !should_propagate {
                event.stop_propagation();
            }
            event.prevent_default();
        },
    )
}

fn install_touchmove(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "touchmove", |event: web_sys::TouchEvent, runner| {
        if let Some((pos, touch)) = primary_touch_pos(runner, &event) {
            if is_interested_in_pointer_event(
                runner,
                egui::pos2(touch.client_x() as f32, touch.client_y() as f32),
            ) {
                let egui_event = egui::Event::PointerMoved(pos);
                let should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
                runner.input.raw.events.push(egui_event);

                push_touches(runner, egui::TouchPhase::Move, &event);
                runner.needs_repaint.repaint_asap();

                // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
                if !should_propagate {
                    event.stop_propagation();
                }
                event.prevent_default();
            }
        }
    })
}

fn install_touchend(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "touchend", |event: web_sys::TouchEvent, runner| {
        if let Some((pos, touch)) = primary_touch_pos(runner, &event) {
            if is_interested_in_pointer_event(
                runner,
                egui::pos2(touch.client_x() as f32, touch.client_y() as f32),
            ) {
                // First release mouse to click:
                let mut should_propagate = false;
                let egui_event = egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: runner.input.raw.modifiers,
                };
                should_propagate |= (runner.web_options.should_propagate_event)(&egui_event);
                runner.input.raw.events.push(egui_event);
                // Then remove hover effect:
                should_propagate |=
                    (runner.web_options.should_propagate_event)(&egui::Event::PointerGone);
                runner.input.raw.events.push(egui::Event::PointerGone);

                push_touches(runner, egui::TouchPhase::End, &event);

                runner.needs_repaint.repaint_asap();

                // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
                if !should_propagate {
                    event.stop_propagation();
                }
                event.prevent_default();

                // Fix virtual keyboard IOS
                // Need call focus at the same time of event
                if runner.text_agent.has_focus() {
                    runner.text_agent.set_focus(false);
                    runner.text_agent.set_focus(true);
                }
            }
        }
    })
}

fn install_touchcancel(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(
        target,
        "touchcancel",
        |event: web_sys::TouchEvent, runner| {
            push_touches(runner, egui::TouchPhase::Cancel, &event);
            event.stop_propagation();
            event.prevent_default();
        },
    )?;

    Ok(())
}

fn install_wheel(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "wheel", |event: web_sys::WheelEvent, runner| {
        let unit = match event.delta_mode() {
            web_sys::WheelEvent::DOM_DELTA_PIXEL => egui::MouseWheelUnit::Point,
            web_sys::WheelEvent::DOM_DELTA_LINE => egui::MouseWheelUnit::Line,
            web_sys::WheelEvent::DOM_DELTA_PAGE => egui::MouseWheelUnit::Page,
            _ => return,
        };

        let delta = -egui::vec2(event.delta_x() as f32, event.delta_y() as f32);

        let modifiers = modifiers_from_wheel_event(&event);

        let egui_event = if modifiers.ctrl && !runner.input.raw.modifiers.ctrl {
            // The browser is saying the ctrl key is down, but it isn't _really_.
            // This happens on pinch-to-zoom on a Mac trackpad.
            // egui will treat ctrl+scroll as zoom, so it all works.
            // However, we explicitly handle it here in order to better match the pinch-to-zoom
            // speed of a native app, without being sensitive to egui's `scroll_zoom_speed` setting.
            let pinch_to_zoom_sensitivity = 0.01; // Feels good on a Mac trackpad in 2024
            let zoom_factor = (pinch_to_zoom_sensitivity * delta.y).exp();
            egui::Event::Zoom(zoom_factor)
        } else {
            egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
            }
        };
        let should_propagate = (runner.web_options.should_propagate_event)(&egui_event);
        runner.input.raw.events.push(egui_event);

        runner.needs_repaint.repaint_asap();

        // Use web options to tell if the web event should be propagated to parent elements based on the egui event.
        if !should_propagate {
            event.stop_propagation();
        }
        event.prevent_default();
    })
}

fn install_drag_and_drop(runner_ref: &WebRunner, target: &EventTarget) -> Result<(), JsValue> {
    runner_ref.add_event_listener(target, "dragover", |event: web_sys::DragEvent, runner| {
        if let Some(data_transfer) = event.data_transfer() {
            runner.input.raw.hovered_items.clear();

            let items = data_transfer.items();
            for i in 0..items.length() {
                if let Some(item) = items.get(i) {
                    match item.kind().as_str() {
                        "file" => {
                            let Ok(Some(file)) = item.get_as_file() else {
                                continue;
                            };
                            runner.input.raw.hovered_items.push(egui::HoveredItem::File(
                                egui::HoveredFile {
                                    mime: file.type_(),
                                    ..Default::default()
                                },
                            ));
                        }
                        "string" => {
                            runner
                                .input
                                .raw
                                .hovered_items
                                .push(egui::HoveredItem::String(egui::HoveredString {
                                    mime: item.type_(),
                                }));
                        }
                        _ => {
                            log::warn!("Unsupported item kind: {:?}", item.kind());
                        }
                    }
                }
            }

            if runner.input.raw.hovered_items.is_empty() {
                // Fallback: just preview anything. Needed on Desktop Safari.
                runner
                    .input
                    .raw
                    .hovered_items
                    .push(egui::HoveredItem::File(egui::HoveredFile::default()));
            }

            // When dragging over, mousemove is not fired.
            let pos = pos_from_mouse_event(runner.canvas(), &event, runner.egui_ctx());
            runner.input.raw.events.push(egui::Event::PointerMoved(pos));

            runner.needs_repaint.repaint_asap();
            event.stop_propagation();
            event.prevent_default();
        }
    })?;

    runner_ref.add_event_listener(target, "dragleave", |event: web_sys::DragEvent, runner| {
        runner.input.raw.hovered_items.clear();
        runner.needs_repaint.repaint_asap();
        event.stop_propagation();
        event.prevent_default();
    })?;

    runner_ref.add_event_listener(target, "drop", {
        let runner_ref = runner_ref.clone();

        move |event: web_sys::DragEvent, runner| {
            if let Some(data_transfer) = event.data_transfer() {
                // TODO(https://github.com/emilk/egui/issues/3702): support dropping folders
                runner.input.raw.hovered_items.clear();
                runner.needs_repaint.repaint_asap();

                let items = data_transfer.items();
                for i in 0..items.length() {
                    if let Some(item) = items.get(i) {
                        match item.kind().as_str() {
                            "file" => {
                                let Ok(Some(file)) = item.get_as_file() else {
                                    continue;
                                };
                                let name = file.name();
                                let mime = file.type_();
                                let last_modified = std::time::UNIX_EPOCH
                                    + std::time::Duration::from_millis(file.last_modified() as u64);

                                log::debug!("Loading {:?} ({} bytes)…", name, file.size());

                                let future =
                                    wasm_bindgen_futures::JsFuture::from(file.array_buffer());

                                let runner_ref = runner_ref.clone();
                                let future = async move {
                                    match future.await {
                                        Ok(array_buffer) => {
                                            let bytes =
                                                js_sys::Uint8Array::new(&array_buffer).to_vec();
                                            log::debug!(
                                                "Loaded {:?} ({} bytes).",
                                                name,
                                                bytes.len()
                                            );

                                            if let Some(mut runner_lock) = runner_ref.try_lock() {
                                                runner_lock.input.raw.dropped_items.push(
                                                    egui::DroppedItem::File(egui::DroppedFile {
                                                        name,
                                                        mime,
                                                        last_modified: Some(last_modified),
                                                        bytes: Some(bytes.into()),
                                                        ..Default::default()
                                                    }),
                                                );
                                                runner_lock.needs_repaint.repaint_asap();
                                            }
                                        }
                                        Err(err) => {
                                            log::error!("Failed to read file: {:?}", err);
                                        }
                                    }
                                };
                                wasm_bindgen_futures::spawn_local(future);
                            }
                            "string" => {
                                let runner_ref = runner_ref.clone();
                                let mime = item.type_();
                                let closure = Closure::once(move |contents: String| {
                                    if let Some(mut runner_lock) = runner_ref.try_lock() {
                                        runner_lock.input.raw.dropped_items.push(
                                            egui::DroppedItem::String(egui::DroppedString {
                                                contents,
                                                mime,
                                            }),
                                        );
                                        runner_lock.needs_repaint.repaint_asap();
                                    }
                                });
                                if let Err(err) = item
                                    .get_as_string(Some(closure.into_js_value().unchecked_ref()))
                                {
                                    log::error!("Failed to read dropped string: {:?}", err);
                                }
                            }
                            _ => {
                                log::warn!("Unsupported item kind: {:?}", item.kind());
                            }
                        }
                    }
                }
                event.stop_propagation();
                event.prevent_default();
            }
        }
    })?;

    // MEMBRANE: Support dragging things natively outside of the canvas.
    runner_ref.add_event_listener(target, "dragstart", |event: web_sys::DragEvent, runner| {
        if let Some(data_transfer) = event.data_transfer() {
            runner.input.raw.native_drag_starting = true;

            let pos = pos_from_mouse_event(runner.canvas(), &event, runner.egui_ctx());
            runner.input.raw.events.push(egui::Event::PointerMoved(pos));

            // Run the logic synchronously to allow the app to set the DragAndDrop payload.
            runner.logic();

            runner.input.raw.native_drag_starting = false;

            if let Some(data) = runner.native_drag_payload.take() {
                if let Err(err) = set_transfer_text(data.as_str(), data_transfer) {
                    log::error!("Failed to set transfer text: {:?}", err);
                }
            } else {
                // Don't let the browser drag the entire canvas
                event.stop_propagation();
                event.prevent_default();
            }
            runner.needs_repaint.repaint_asap();
        }
    })?;

    // MEMBRANE: Support dragging things natively outside of the canvas.
    runner_ref.add_event_listener(target, "dragend", |event: web_sys::DragEvent, runner| {
        runner.input.raw.hovered_items.clear();
        if let Some(button) = button_from_mouse_event(&event) {
            // Send this event because pointerup is not fired when dragging
            let pos = pos_from_mouse_event(runner.canvas(), &event, runner.egui_ctx());
            let modifiers = runner.input.raw.modifiers;
            let events = &mut runner.input.raw.events;
            events.push(egui::Event::PointerButton {
                pos,
                button,
                pressed: false,
                modifiers,
            });
            runner.needs_repaint.repaint_asap();
            clean_up_transfer_text();
        }
    })?;

    Ok(())
}

fn set_transfer_text(data: &str, data_transfer: web_sys::DataTransfer) -> Result<(), &'static str> {
    let window = web_sys::window().ok_or("Failed to get window")?;
    let document = window.document().ok_or("Failed to get document")?;
    let element = document
        .create_element("div")
        .map_err(|_| "Failed to create element")?;
    element.set_id("eframe-dragged-element");
    document
        .body()
        .ok_or("Failed to get document body")?
        .append_child(&element)
        .map_err(|_| "Failed to append child element")?;
    element.set_text_content(Some(data));
    data_transfer
        .set_data("text/plain", &data)
        .map_err(|_| "Failed to set data transfer")?;
    data_transfer.set_drag_image(&element, 0, 0);
    Ok(())
}

fn clean_up_transfer_text() {
    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    if let Some(element) = document.get_element_by_id("eframe-dragged-element") {
        element.remove();
    }
}

/// Install a `ResizeObserver` to observe changes to the size of the canvas.
///
/// This is the only way to ensure a canvas size change without an associated window `resize` event
/// actually results in a resize of the canvas.
///
/// The resize observer is called the by the browser at `observe` time, instead of just on the first actual resize.
/// We use that to trigger the first `request_animation_frame` _after_ updating the size of the canvas to the correct dimensions,
/// to avoid [#4622](https://github.com/emilk/egui/issues/4622).
pub(crate) fn install_resize_observer(runner_ref: &WebRunner) -> Result<(), JsValue> {
    let closure = Closure::wrap(Box::new({
        let runner_ref = runner_ref.clone();
        move |entries: js_sys::Array| {
            // Only call the wrapped closure if the egui code has not panicked
            if let Some(mut runner_lock) = runner_ref.try_lock() {
                let canvas = runner_lock.canvas();
                let (width, height) = match get_display_size(&entries) {
                    Ok(v) => v,
                    Err(err) => {
                        log::error!("{}", super::string_from_js_value(&err));
                        return;
                    }
                };
                canvas.set_width(width);
                canvas.set_height(height);

                // force an immediate repaint
                runner_lock.needs_repaint.repaint_asap();
                paint_if_needed(&mut runner_lock);
                drop(runner_lock);
                // we rely on the resize observer to trigger the first `request_animation_frame`:
                if let Err(err) = runner_ref.request_animation_frame() {
                    log::error!("{}", super::string_from_js_value(&err));
                };
            }
        }
    }) as Box<dyn FnMut(js_sys::Array)>);

    let observer = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref())?;
    let options = web_sys::ResizeObserverOptions::new();
    options.set_box(web_sys::ResizeObserverBoxOptions::ContentBox);
    if let Some(runner_lock) = runner_ref.try_lock() {
        observer.observe_with_options(runner_lock.canvas(), &options);
        drop(runner_lock);
        runner_ref.set_resize_observer(observer, closure);
    }

    Ok(())
}

// Code ported to Rust from:
// https://webglfundamentals.org/webgl/lessons/webgl-resizing-the-canvas.html
fn get_display_size(resize_observer_entries: &js_sys::Array) -> Result<(u32, u32), JsValue> {
    let width;
    let height;
    let mut dpr = web_sys::window().unwrap().device_pixel_ratio();

    let entry: web_sys::ResizeObserverEntry = resize_observer_entries.at(0).dyn_into()?;
    if JsValue::from_str("devicePixelContentBoxSize").js_in(entry.as_ref()) {
        // NOTE: Only this path gives the correct answer for most browsers.
        // Unfortunately this doesn't work perfectly everywhere.
        let size: web_sys::ResizeObserverSize =
            entry.device_pixel_content_box_size().at(0).dyn_into()?;
        width = size.inline_size();
        height = size.block_size();
        dpr = 1.0; // no need to apply
    } else if JsValue::from_str("contentBoxSize").js_in(entry.as_ref()) {
        let content_box_size = entry.content_box_size();
        let idx0 = content_box_size.at(0);
        if !idx0.is_undefined() {
            let size: web_sys::ResizeObserverSize = idx0.dyn_into()?;
            width = size.inline_size();
            height = size.block_size();
        } else {
            // legacy
            let size = JsValue::clone(content_box_size.as_ref());
            let size: web_sys::ResizeObserverSize = size.dyn_into()?;
            width = size.inline_size();
            height = size.block_size();
        }
    } else {
        // legacy
        let content_rect = entry.content_rect();
        width = content_rect.width();
        height = content_rect.height();
    }

    Ok(((width.round() * dpr) as u32, (height.round() * dpr) as u32))
}
