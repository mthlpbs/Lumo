//! Choose a page only when the person opens the closed island. Explicit
//! destinations bypass this policy; an already open page never jumps away.

use crate::activities::{self, Activity};
use crate::pages::PageKind;
use crate::state::AppState;

/// Reading, scrolling or operating a page counts as continuing a task for
/// 90 seconds. Merely opening a tab or hovering must not make it sticky.
/// Checked on input/open only: expiry needs no timer or background work.
const CONTINUE_S: f64 = 90.0;

#[derive(Default)]
pub struct Context {
    interaction: Option<(PageKind, f64)>,
}

impl Context {
    pub fn selected(&mut self) {
        self.interaction = None;
    }

    pub fn interacted(&mut self, page: PageKind, now: f64) {
        self.interaction = Some((page, now));
    }

    pub fn page(&self, st: &AppState, now: f64) -> PageKind {
        let available = |page| match page {
            PageKind::Ai => st.ai_on(),
            PageKind::Settings => false,
            PageKind::Script => st.script_on(),
            PageKind::Clipboard => st.settings.clipboard_history,
            _ => true,
        };
        let last = if available(st.page) {
            st.page
        } else {
            st.ui.before_settings.filter(|p| available(*p)).unwrap_or(PageKind::Media)
        };

        // Preserve unfinished input only on the page the person left. A
        // forgotten draft elsewhere must never override an intentional switch.
        let unfinished = match last {
            PageKind::Ai => !st.ai.full_open && (!st.ai.draft.trim().is_empty()
                || !st.ai.pending.is_empty() || st.ai.attaching || st.ai.snipping),
            PageKind::Tasks => !st.ui.task_input.trim().is_empty(),
            PageKind::Shelf => st.phone.as_ref().is_some_and(|p| p.holds_island()),
            PageKind::Script => st.script.active,
            _ => false,
        };
        // Sending hands the task to AI. Waiting for it is background work,
        // even if Send or a scroll was the last interaction with the page.
        let can_continue = last != PageKind::Ai || (!st.ai.busy && !st.ai.full_open);
        let recent = can_continue && self.interaction.is_some_and(|(page, at)|
            page == last && now >= at && now - at <= CONTINUE_S);
        if unfinished || recent {
            return last;
        }

        if st.phone.as_ref().is_some_and(|p| p.ask.is_some() || p.transfer.is_some()) {
            return PageKind::Shelf;
        }
        if st.script_on() && st.script.active {
            return PageKind::Script;
        }
        match activities::current_activity(st) {
            Some(Activity::Game) => PageKind::System,
            Some(Activity::Media) => PageKind::Media,
            Some(Activity::Timer) => PageKind::Focus,
            // Both controls are useful: keep the one already being used.
            Some(Activity::MediaTimer) if last == PageKind::Focus => PageKind::Focus,
            Some(Activity::MediaTimer) => PageKind::Media,
            None => last,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MediaSession;
    use lumo_core::{settings::Settings, store::Store, timeline::Timeline};

    fn music() -> AppState {
        let mut st = AppState::new(Settings::default(), Store::default());
        st.settings.set_addon("ai", true);
        st.now = 100.0;
        st.page = PageKind::Ai;
        st.media = Some(MediaSession {
            app_id: "player".into(), app_name: "Player".into(), music_source: None,
            title: "Track".into(), artist: String::new(), album: String::new(),
            art: None, art_color: None, timeline: Timeline::new(0.0, 300.0, st.now, true),
            can_prev: true, can_next: true, can_play_pause: true, can_seek: true,
            track_key: "track".into(),
        });
        st
    }

    #[test]
    fn music_replaces_a_passively_opened_chat_and_other_idle_pages() {
        let mut st = music();
        let context = Context::default();
        for page in PageKind::ALL {
            st.page = page;
            assert_eq!(context.page(&st, 100.0), PageKind::Media, "{page:?}");
        }
    }

    #[test]
    fn unfinished_input_is_resumed_until_explicitly_switching_away() {
        let mut st = music();
        let context = Context::default();
        st.ai.draft = "Follow up".into();
        assert_eq!(context.page(&st, 500.0), PageKind::Ai);
        st.ai.busy = true;
        assert_eq!(context.page(&st, 500.0), PageKind::Ai, "a new draft is still work");
        st.page = PageKind::Tasks;
        assert_eq!(context.page(&st, 500.0), PageKind::Media, "a hidden AI draft cannot steal focus");
        st.ui.task_input = "Finish this task".into();
        assert_eq!(context.page(&st, 500.0), PageKind::Tasks);
        st.ui.task_input = " \n ".into();
        assert_eq!(context.page(&st, 500.0), PageKind::Media);
    }

    #[test]
    fn real_interaction_resumes_but_sending_yields_to_music() {
        let mut st = music();
        let mut context = Context::default();
        context.interacted(PageKind::Ai, 100.0);
        assert_eq!(context.page(&st, 110.0), PageKind::Ai);
        assert_eq!(context.page(&st, 191.0), PageKind::Media);
        st.ai.busy = true;
        assert_eq!(context.page(&st, 110.0), PageKind::Media);
        st.ai.busy = false;
        st.ai.full_open = true;
        assert_eq!(context.page(&st, 110.0), PageKind::Media);
        st.ai.full_open = false;
        st.page = PageKind::Tasks;
        context.interacted(st.page, 100.0);
        assert_eq!(context.page(&st, 110.0), PageKind::Tasks);
        assert_eq!(context.page(&st, 191.0), PageKind::Media);
        context.selected();
        assert_eq!(context.page(&st, 110.0), PageKind::Media, "switching tabs ends the previous continuation");
    }

    #[test]
    fn timer_game_and_disabled_addons_choose_available_destinations() {
        let mut st = music();
        let context = Context::default();
        st.pomodoro.start(st.now);
        assert_eq!(context.page(&st, 100.0), PageKind::Media);
        st.page = PageKind::Focus;
        assert_eq!(context.page(&st, 100.0), PageKind::Focus);
        st.media = None;
        st.page = PageKind::Ai;
        st.ai.busy = true;
        assert_eq!(context.page(&st, 100.0), PageKind::Focus);
        st.game.active = true;
        assert_eq!(context.page(&st, 100.0), PageKind::System);
        st.game.active = false;
        st.pomodoro.reset();
        st.settings.set_addon("ai", false);
        st.ai.draft = "Saved draft".into();
        for page in [PageKind::Ai, PageKind::Clipboard, PageKind::Script, PageKind::Settings] {
            st.page = page;
            assert_eq!(context.page(&st, 100.0), PageKind::Media);
        }
        st.page = PageKind::Tasks;
        assert_eq!(context.page(&st, 100.0), PageKind::Tasks);
    }

    #[test]
    fn phone_transfers_script_reading_and_pause_grace_follow_live_context() {
        let mut st = music();
        let context = Context::default();
        st.phone = Some(crate::state::PhoneDrop {
            transfer: Some(crate::state::PhoneTransfer {
                id: 1, from: "Phone".into(), file: 0, files: 1, done: 10, total: 100,
            }),
            ..Default::default()
        });
        assert_eq!(context.page(&st, 100.0), PageKind::Shelf);
        st.phone = None;
        st.addons.insert("script", lumo_core::addons::Status::Ready);
        st.script.active = true;
        assert_eq!(context.page(&st, 100.0), PageKind::Script);
        st.script.active = false;
        st.media.as_mut().unwrap().timeline.playing = false;
        st.media_paused_at = Some(95.0);
        assert_eq!(context.page(&st, 100.0), PageKind::Media);
        st.now = 126.0;
        assert_eq!(context.page(&st, 126.0), PageKind::Ai, "an expired paused activity cannot hide the last page");
        st.media.as_mut().unwrap().timeline.playing = true;
        st.settings.show_media_activity = false;
        assert_eq!(context.page(&st, 126.0), PageKind::Ai, "respect hidden media activity");
    }
}
