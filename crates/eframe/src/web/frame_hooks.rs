/// Callbacks invoked around each `requestAnimationFrame` callback, owned by the [`super::WebRunner`].
///
/// Used to bracket all work that happens within a single animation-frame invocation (e.g. to tag
/// log events with the frame they occurred in). Register via [`super::WebRunner::on_frame_begin`]
/// and [`super::WebRunner::on_frame_end`].
#[derive(Default)]
pub(crate) struct FrameHooks {
    begin: Vec<Box<dyn Fn(u64)>>,
    end: Vec<Box<dyn Fn()>>,
}

impl FrameHooks {
    pub fn push_begin(&mut self, callback: Box<dyn Fn(u64)>) {
        self.begin.push(callback);
    }

    pub fn push_end(&mut self, callback: Box<dyn Fn()>) {
        self.end.push(callback);
    }

    pub fn run_begin(&self, frame_nr: u64) {
        for hook in &self.begin {
            hook(frame_nr);
        }
    }

    pub fn run_end(&self) {
        for hook in &self.end {
            hook();
        }
    }
}
