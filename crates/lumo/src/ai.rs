//! Companion state belongs to the UI. All disk/network/attachment work is on demand.
use crate::{
    commands::Command,
    gfx::Image,
    icons::Icon,
    state::{AppState, NoticeAction},
};
use lumo_core::{
    ai::{Attachment, Chat, Message, Model},
    geom::palette,
};
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Load,
    CheckKey,
    Previews,
    EnsureModels,
    FetchModels,
    SaveKey,
    ClearKey,
    Send,
    Retry,
    Stop,
    New(bool),
    ToggleTemporary,
    Select(u64),
    Delete(u64),
    DeleteAll,
    SaveHistory,
    Attach(bool), // true: images; false: PDF/text documents
    AttachFiles(Vec<String>),
    PasteAttachment,
    CopyImage(usize, usize),
    DownloadImage(usize, usize),
    ReuseImage(usize, usize),
}
pub type ProviderKey = lumo_core::ai::Provider;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Generation {
    #[default]
    Idle,
    Working,
    Ready,
    Stopped,
    Failed,
}
pub struct ReadyReply {
    pub chat: u64,
    pub preview: String,
}
pub enum ModelRow {
    Model(usize),
}
pub struct Catalog {
    pub models: Vec<Model>,
    pub saved_at: u64,
    pub from_cache: bool,
    pub warning: Option<String>,
}
pub enum Event {
    Loaded(Vec<Chat>, ProviderKey, bool),
    KeyChecked(ProviderKey, bool),
    Models(String, Result<Catalog, String>),
    Reply {
        request: u64,
        chat: u64,
        result: Result<Message, String>,
        images: Vec<Image>,
        unavailable: Option<(String, String)>, // catalog scope and requested ID
    },
    Previews(u64, Vec<(usize, usize, Image)>),
    Attached {
        result: Result<Vec<Attachment>, String>,
        previews: Vec<Option<Image>>,
    },
    Saved(Result<(), String>),
    KeySaved(ProviderKey, bool, Result<(), String>),
    WorkerFailed(String),
}
pub struct State {
    pub sites: crate::ai_sites::State,
    pub return_page: Option<crate::pages::PageKind>,
    pub fresh_on_open: bool,
    pub island_height: f32,
    pub max_island_height: f32,
    pub canvas_height: f32,
    pub resize_from: f32,
    pub resize_dragged: bool,
    pub scroll_to_reply: Option<usize>,
    pub auto_fetch_key: String,
    pub loaded: bool,
    pub loading: bool,
    pub history_cleared: bool,
    pub chats: Vec<Chat>,
    pub current: Chat,
    pub temporary: bool,
    pub draft: String,
    pub pending: Vec<Attachment>,
    pub pending_previews: Vec<Option<Image>>,
    pub models: Vec<Model>,
    pub unavailable_models: std::collections::HashSet<(String, String)>,
    pub catalog_key: String,
    pub fetching: bool,
    pub catalog_saved_at: u64,
    pub catalog_from_cache: bool,
    pub attaching: bool,
    pub config_editing: bool,
    pub key_draft: String,
    pub key_present: bool,
    pub key_visible: bool,
    pub key_saving: bool,
    pub cancel: Option<std::sync::Arc<crate::services::http::Cancel>>,
    pub selected: String,
    pub busy: bool,
    pub generation: Generation,
    pub generation_started: f64,
    pub generation_finished: f64,
    pub ready_reply: Option<ReadyReply>,
    pub request: u64,
    pub running_chat: u64,
    pub error: String,
    pub model_menu: bool,
    pub reasoning_menu: bool,
    pub model_detail: Option<String>,
    pub model_detail_scroll: f32,
    pub catalog_revision: u64,
    pub model_rows: Vec<ModelRow>,
    pub model_rows_key: Option<(u64, usize, String, u8)>,
    pub model_filters_open: bool,
    /// List height in px, frozen while the filter popover is open.
    pub model_filter_rows: usize,
    pub model_filters: u8,
    pub model_settings: bool,
    pub search: String,
    pub model_scroll: f32,
    /// Keyboard-highlighted row of `model_rows`. Cleared when the pointer moves.
    pub model_cursor: Option<usize>,
    pub model_cursor_mouse: Option<lumo_core::geom::Point>,
    pub reveal_model: bool,
    pub hover_model: Option<(String, f64)>,
    pub attach_menu: bool,
    pub history: bool,
    pub history_query: String,
    pub history_scroll: f32,
    pub scroll: f32,
    pub max_scroll: f32,
    pub reply_at: f64,
    pub previews: std::collections::HashMap<(usize, usize), Image>,
    pub full_open: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            sites: Default::default(),
            return_page: None,
            fresh_on_open: false,
            island_height: 680.0,
            max_island_height: 840.0,
            canvas_height: 680.0,
            resize_from: 240.0,
            resize_dragged: false,
            scroll_to_reply: None,
            auto_fetch_key: String::new(),
            loaded: false,
            loading: false,
            history_cleared: false,
            chats: vec![],
            current: Chat::default(),
            temporary: false,
            draft: String::new(),
            pending: vec![],
            pending_previews: vec![],
            models: vec![],
            unavailable_models: Default::default(),
            catalog_key: String::new(),
            fetching: false,
            catalog_saved_at: 0,
            catalog_from_cache: false,
            attaching: false,
            config_editing: false,
            key_draft: String::new(),
            key_present: false,
            key_visible: false,
            key_saving: false,
            cancel: None,
            selected: String::new(),
            busy: false,
            generation: Generation::Idle,
            generation_started: 0.0,
            generation_finished: 0.0,
            ready_reply: None,
            request: 0,
            running_chat: 0,
            error: String::new(),
            model_menu: false,
            reasoning_menu: false,
            model_detail: None,
            model_detail_scroll: 0.0,
            catalog_revision: 0,
            model_rows: vec![],
            model_rows_key: None,
            model_filters_open: false,
            model_filter_rows: 0,
            model_filters: 0,
            model_settings: false,
            search: String::new(),
            model_scroll: 0.0,
            model_cursor: None,
            model_cursor_mouse: None,
            reveal_model: true,
            hover_model: None,
            attach_menu: false,
            history: false,
            history_query: String::new(),
            history_scroll: 0.0,
            scroll: 0.0,
            max_scroll: 0.0,
            reply_at: 0.0,
            previews: Default::default(),
            full_open: false,
        }
    }
}
impl State {
    /// Closing with Resume off releases unsent attachment/image buffers now;
    /// keep the current chat identity until reopening so an explicit in-flight
    /// request can still deliver its reply and notification normally.
    pub fn close_workspace(&mut self, resume: bool) {
        self.close_popovers();
        self.fresh_on_open = !resume;
        if !resume {
            self.draft.clear();
            self.pending.clear();
            self.pending_previews.clear();
            self.previews.clear();
            self.history_query.clear();
            self.history = false;
            self.scroll = 0.0;
            self.max_scroll = 0.0;
        }
    }
    pub fn close_models(&mut self) {
        self.model_filters_open = false;
        self.model_filter_rows = 0;
        self.model_cursor = None;
        self.model_menu = false;
        self.reasoning_menu = false;
        self.model_detail = None;
        self.hover_model = None;
    }
    pub fn close_popovers(&mut self) {
        self.sites.menu = false;
        self.close_models();
        self.attach_menu = false;
    }
    pub fn snapshot(&mut self) {
        if self.temporary || self.current.messages.is_empty() {
            return;
        }
        if let Some(c) = self.chats.iter_mut().find(|c| c.id == self.current.id) {
            *c = self.current.clone();
        } else {
            self.chats.insert(0, self.current.clone());
            self.chats.truncate(40);
        }
    }
    pub fn model(&self, config: &lumo_core::ai::Config) -> Model {
        let id = if self.selected.is_empty() {
            &config.default_model
        } else {
            &self.selected
        };
        let mut model = self
            .models
            .iter()
            .find(|m| &m.id == id)
            .cloned()
            .unwrap_or_else(|| Model::manual(id));
        model.reasoning.resolve(&model.id, config.provider);
        if !id.is_empty()
            && lumo_core::ai::is_hackclub(config.models())
            && self.catalog_key == config_catalog_key(config)
            && !self.models.is_empty()
            && !self.models.iter().any(|m| &m.id == id)
        {
            model.unavailable = Some(
                "Not listed in the saved Hack Club AI catalog. Refresh the model list.".into(),
            );
        }
        if self
            .unavailable_models
            .contains(&(config_catalog_key(config), model.id.clone()))
        {
            model.unavailable =
                Some("Unavailable for this endpoint. Refresh the model list to try again.".into());
        }
        model
    }
    pub fn model_unavailable(&self, config: &lumo_core::ai::Config, id: &str) -> Option<String> {
        if self
            .unavailable_models
            .contains(&(config_catalog_key(config), id.to_owned()))
        {
            Some("Unavailable for this endpoint. Refresh the model list to try again.".into())
        } else {
            self.models
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.unavailable.clone())
        }
    }
    pub fn initialize(&mut self) {
        if !self.loaded && !self.loading {
            self.loading = true;
        }
    }
    /// Escape dismisses the most local surface before its parent workspace.
    pub fn dismiss_popup(&mut self) {
        if self.model_filters_open {
            self.model_filters_open = false;
        } else if self.model_detail.is_some() {
            self.model_detail = None;
            self.model_detail_scroll = 0.0;
        } else {
            self.close_popovers();
        }
    }
}
pub fn ensure(st: &mut AppState) {
    if !st.ai_on() {
        return;
    }
    if !st.ai.loaded && !st.ai.loading {
        st.ai.initialize();
        st.cmd(Command::Ai(Action::Load));
    }
    let scope = catalog_key(st);
    if !st.ai.config_editing
        && (!st.settings.ai.models_url.trim().is_empty() || st.ai.key_present)
        && st.ai.auto_fetch_key != scope
        && lumo_core::ai::validate_url(st.settings.ai.models()).is_ok()
    {
        st.ai.auto_fetch_key = scope;
        st.cmd(Command::Ai(Action::EnsureModels));
    }
}

/// The same shortcuts in the island and mini host. They only change local
/// UI state; opening an empty model catalog keeps its existing on-demand path.
pub fn workspace_key(
    st: &mut AppState,
    key: u32,
    ctrl: bool,
    shift: bool,
) -> Option<crate::ui::WidgetId> {
    use crate::ui::wid;
    if !ctrl || st.ai.sites.selected.is_some() {
        return None;
    }
    match key {
        0x4e => {
            st.cmd(Command::Ai(Action::New(shift)));
            Some(wid("ai.prompt"))
        }
        0x48 => {
            st.ai.close_popovers();
            st.ai.history = !st.ai.history;
            Some(wid(if st.ai.history {
                "ai.history-filter"
            } else {
                "ai.prompt"
            }))
        }
        0x4b => {
            let open = !st.ai.model_menu;
            st.ai.close_popovers();
            st.ai.model_menu = open;
            st.ai.model_settings = false;
            st.ai.reveal_model = true;
            st.ai.model_scroll = 0.0;
            if open && st.ai.models.is_empty() {
                st.cmd(Command::Ai(Action::EnsureModels));
            }
            Some(wid(if open { "ai.model-search" } else { "ai.prompt" }))
        }
        _ => None,
    }
}
pub fn apply(st: &mut AppState, event: Event, notify: bool) {
    match event {
        Event::KeyChecked(p, key) => {
            if st.settings.ai.provider == p {
                st.ai.key_present = key;
            }
        }
        Event::Loaded(chats, provider, key) => {
            st.ai.loading = false;
            st.ai.loaded = true;
            if provider == st.settings.ai.provider {
                st.ai.key_present = key;
            }
            // A startup read may finish after Delete all. Never restore those chats.
            if !st.ai.history_cleared {
                for chat in chats {
                    if !st.ai.chats.iter().any(|c| c.id == chat.id) {
                        st.ai.chats.push(chat);
                    }
                }
            }
            st.ai.chats.truncate(40);
            if st.ai.current.messages.is_empty() {
                st.ai.current = Chat {
                    id: next_id(),
                    ..Default::default()
                };
            }
        }
        Event::Models(key, r) => {
            if key != catalog_key(st) {
                return;
            }
            st.ai.fetching = false;
            match r {
                Ok(catalog) => {
                    if !catalog.from_cache {
                        st.ai.unavailable_models.retain(|(scope, _)| scope != &key);
                    }
                    let mut models = catalog.models;
                    // Hack Club documents this endpoint as its complete live
                    // list. Preserve saved selections as disabled rows when
                    // a successful refresh confirms they have disappeared.
                    if lumo_core::ai::is_hackclub(st.settings.ai.models()) && !catalog.from_cache {
                        for id in [&st.ai.selected, &st.settings.ai.default_model] {
                            if !id.is_empty() && !models.iter().any(|m| &m.id == id) {
                                let mut m = Model::manual(id);
                                m.unavailable =
                                    Some("Not listed in Hack Club AI's current catalog".into());
                                models.push(m);
                            }
                        }
                    }
                    st.ai.models = models;
                    st.ai.catalog_saved_at = catalog.saved_at;
                    st.ai.catalog_from_cache = catalog.from_cache;
                    st.ai.catalog_revision = st.ai.catalog_revision.wrapping_add(1);
                    st.ai.catalog_key = key;
                    st.ai.error = catalog.warning.unwrap_or_default();
                }
                Err(e) => st.ai.error = e,
            }
        }
        Event::Reply {
            request,
            chat,
            result,
            images,
            unavailable,
        } => {
            if request != st.ai.request || !st.ai.busy {
                return;
            }
            st.ai.busy = false;
            st.ai.cancel = None;
            st.ai.generation_finished = st.now;
            if let Some((scope, id)) = unavailable {
                if st.ai.unavailable_models.len() < 5000 {
                    st.ai.unavailable_models.insert((scope, id));
                    st.ai.catalog_revision = st.ai.catalog_revision.wrapping_add(1);
                }
            }
            match result {
                Ok(message) => {
                    let preview = lumo_core::ai::plain_text(&message.text)
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("Your image is ready")
                        .chars()
                        .take(90)
                        .collect::<String>();
                    let delivered =
                        st.ai.current.id == chat || st.ai.chats.iter().any(|c| c.id == chat);
                    st.ai.generation = if delivered {
                        Generation::Ready
                    } else {
                        Generation::Idle
                    };
                    if st.ai.current.id == chat {
                        st.ai.current.messages.push(message);
                        let mi = st.ai.current.messages.len() - 1;
                        st.ai.scroll_to_reply = Some(mi);
                        for (i, img) in images.into_iter().enumerate() {
                            st.ai.previews.insert((mi, i), img);
                        }
                        st.ai.reply_at = st.now;
                        st.ai.scroll = f32::MAX;
                        st.ai.snapshot();
                    } else if let Some(c) = st.ai.chats.iter_mut().find(|c| c.id == chat) {
                        c.messages.push(message);
                    }
                    st.cmd(Command::Ai(Action::SaveHistory));
                    if notify && delivered {
                        st.ai.ready_reply = Some(ReadyReply { chat, preview });
                        announce_ready(st);
                    }
                }
                Err(e) => {
                    st.ai.generation = Generation::Failed;
                    st.ai.error = e;
                }
            }
        }
        Event::Previews(chat, images) => {
            if chat == st.ai.current.id {
                st.ai.previews = images.into_iter().map(|(m, a, i)| ((m, a), i)).collect();
            }
        }
        Event::Attached {
            result: r,
            previews,
        } => {
            st.ai.attaching = false;
            match r {
                Ok(a) => {
                    let size: usize = st
                        .ai
                        .pending
                        .iter()
                        .chain(a.iter())
                        .map(|a| a.data.len())
                        .sum();
                    if st.ai.pending.len() + a.len() > 4 || size > 24 * 1024 * 1024 {
                        st.ai.error =
                            "Attach up to 4 files, with a combined size under 18 MB.".into();
                    } else {
                        st.ai
                            .pending_previews
                            .resize_with(st.ai.pending.len(), || None);
                        st.ai.pending_previews.extend(
                            previews
                                .into_iter()
                                .chain(std::iter::repeat(None))
                                .take(a.len()),
                        );
                        st.ai.pending.extend(a);
                        st.ai.error.clear();
                    }
                }
                Err(e) => st.ai.error = e,
            }
        }
        Event::Saved(r) => {
            if let Err(e) = r {
                st.ai.error = e;
            }
        }
        Event::WorkerFailed(error) => {
            if st.ai.busy {
                st.ai.generation = Generation::Failed;
                st.ai.generation_finished = st.now;
            }
            st.ai.busy = false;
            st.ai.fetching = false;
            st.ai.loading = false;
            st.ai.key_saving = false;
            st.ai.attaching = false;
            st.ai.error = error;
        }
        Event::KeySaved(provider, present, r) => {
            st.ai.key_saving = false;
            if provider != st.settings.ai.provider {
                return;
            }
            match r {
                Ok(()) => {
                    st.ai.key_present = present;
                    st.ai.key_draft.clear();
                    st.ai.error.clear();
                    if present {
                        st.cmd(Command::Ai(Action::EnsureModels));
                    }
                }
                Err(e) => st.ai.error = e,
            }
        }
    }
}
pub fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64)
        .saturating_add(SEQ.fetch_add(1, Ordering::Relaxed))
}
pub fn catalog_key(st: &AppState) -> String {
    config_catalog_key(&st.settings.ai)
}
fn config_catalog_key(config: &lumo_core::ai::Config) -> String {
    format!(
        "{:?}|{}|{}",
        config.provider,
        config.completion(),
        config.models()
    )
}
pub fn paste_attachment(
    st: &mut AppState,
    prompt_focused: bool,
    has_attachment: impl FnOnce() -> bool,
) -> bool {
    if st.ai.sites.selected.is_some() || !prompt_focused || !has_attachment() {
        return false;
    }
    st.ai.close_popovers();
    st.cmd(Command::Ai(Action::PasteAttachment));
    true
}
pub fn local(st: &mut AppState, a: &Action) -> bool {
    match a {
        Action::ToggleTemporary => {
            st.ai.reasoning_menu = false;
            st.ai.model_menu = false;
            st.ai.model_detail = None;
            let temporary = !st.ai.temporary;
            if st.ai.current.messages.is_empty() {
                st.ai.temporary = temporary;
                st.ai.history = false;
            } else {
                // Start a separate conversation at the privacy boundary;
                // turning temporary off must never save its private messages.
                // Unsent work and the chosen model belong to the person.
                let draft = std::mem::take(&mut st.ai.draft);
                let pending = std::mem::take(&mut st.ai.pending);
                let previews = std::mem::take(&mut st.ai.pending_previews);
                let selected = std::mem::take(&mut st.ai.selected);
                local(st, &Action::New(temporary));
                st.ai.draft = draft;
                st.ai.pending = pending;
                st.ai.pending_previews = previews;
                st.ai.selected = selected;
            }
        }
        Action::New(temp) => {
            st.ai.reasoning_menu = false;
            st.ai.model_menu = false;
            st.ai.model_detail = None;
            if !st.ai.busy {
                st.ai.generation = Generation::Idle;
            }
            st.ai.scroll_to_reply = None;
            st.ai.snapshot();
            st.ai.selected = st.settings.ai.default_model.clone();
            st.ai.current = Chat {
                id: next_id(),
                ..Default::default()
            };
            st.ai.temporary = *temp;
            st.ai.draft.clear();
            st.ai.pending.clear();
            st.ai.pending_previews.clear();
            st.ai.previews.clear();
            st.ai.error.clear();
            st.ai.history = false;
            st.ai.scroll = 0.0;
            st.cmd(Command::Ai(Action::SaveHistory));
        }
        Action::Select(id) => {
            st.ai.snapshot();
            if let Some(c) = st.ai.chats.iter().find(|c| c.id == *id).cloned() {
                st.ai.current = c;
                st.ai.temporary = false;
                st.ai.selected = st.ai.current.model.clone();
                st.ai.previews.clear();
                st.ai.history = false;
                st.ai.scroll = f32::MAX;
                st.ai.scroll_to_reply = st
                    .ai
                    .current
                    .messages
                    .iter()
                    .rposition(|m| m.role == "assistant");
                st.cmd(Command::Ai(Action::Previews));
            }
        }
        Action::Delete(id) => {
            st.ai.chats.retain(|c| c.id != *id);
            if st.ai.current.id == *id {
                st.ai.current = Chat {
                    id: next_id(),
                    ..Default::default()
                };
                st.ai.previews.clear();
            }
            st.cmd(Command::Ai(Action::SaveHistory));
        }
        Action::DeleteAll => {
            st.ai.history_cleared = true;
            st.ai.chats.clear();
            st.ai.current = Chat {
                id: next_id(),
                ..Default::default()
            };
            st.ai.previews.clear();
            st.cmd(Command::Ai(Action::SaveHistory));
        }
        Action::Stop => {
            if let Some(c) = st.ai.cancel.take() {
                c.cancel();
            }
            st.ai.request = st.ai.request.wrapping_add(1);
            st.ai.busy = false;
            st.ai.generation = Generation::Stopped;
            st.ai.generation_finished = st.now;
            st.ai.error =
                "Request cancelled. Your provider may charge for work already completed.".into();
        }
        Action::ReuseImage(mi, ai) => {
            if let Some(a) = st
                .ai
                .current
                .messages
                .get(*mi)
                .and_then(|m| m.attachments.get(*ai))
                .cloned()
            {
                if st.ai.pending.len() < 4 {
                    st.ai
                        .pending_previews
                        .resize_with(st.ai.pending.len(), || None);
                    st.ai
                        .pending_previews
                        .push(st.ai.previews.get(&(*mi, *ai)).cloned());
                    st.ai.pending.push(a);
                }
                st.ai.attach_menu = false;
            }
        }
        _ => return false,
    }
    true
}

/// Reuse the notice surface; no polling, background animation or new timer.
pub fn announce_ready(st: &mut AppState) {
    let Some(ready) = st.ai.ready_reply.as_ref() else {
        return;
    };
    let (chat, preview) = (ready.chat, ready.preview.clone());
    st.notify(
        Icon::Sparkles,
        palette::BLUE,
        "AI reply ready",
        preview,
        0.0,
        Some(NoticeAction::OpenAiReply(chat)),
    );
    st.set_notice_inline(crate::state::InlineAction::Hide);
}
pub fn clear_ready(st: &mut AppState) {
    st.ai.ready_reply = None;
    if st.notice.as_ref().is_some_and(|n| n.is_ai_reply()) {
        st.notice = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn escape_unwinds_filter_then_details_then_picker() {
        let mut s = State::default();
        s.model_menu = true;
        s.model_filters_open = true;
        s.dismiss_popup();
        assert!(s.model_menu && !s.model_filters_open);
        s.model_detail = Some("model".into());
        s.dismiss_popup();
        assert!(s.model_menu && s.model_detail.is_none());
        s.dismiss_popup();
        assert!(!s.model_menu);
    }
    #[test]
    fn model_refusal_stays_with_requested_endpoint_and_refresh_can_recover() {
        let mut st = state();
        st.settings.ai.provider = lumo_core::ai::Provider::Compatible;
        st.settings.ai.completion_url = "https://example.org/chat".into();
        let scope = catalog_key(&st);
        st.ai.selected = "chosen-later".into();
        st.ai.busy = true;
        st.ai.request = 7;
        apply(
            &mut st,
            Event::Reply {
                request: 7,
                chat: 17,
                result: Err("Model refused".into()),
                images: vec![],
                unavailable: Some((scope.clone(), "failed-model".into())),
            },
            false,
        );
        assert!(st
            .ai
            .model_unavailable(&st.settings.ai, "failed-model")
            .is_some());
        assert!(st.ai.model(&st.settings.ai).unavailable.is_none());
        st.settings.ai.completion_url = "https://elsewhere.org/chat".into();
        assert!(st
            .ai
            .model_unavailable(&st.settings.ai, "failed-model")
            .is_none());
        st.settings.ai.completion_url = "https://example.org/chat".into();
        let catalog = |from_cache| Catalog {
            models: vec![Model::manual("failed-model")],
            saved_at: 1,
            from_cache,
            warning: None,
        };
        apply(
            &mut st,
            Event::Models(scope.clone(), Ok(catalog(true))),
            false,
        );
        assert!(st
            .ai
            .model_unavailable(&st.settings.ai, "failed-model")
            .is_some());
        apply(
            &mut st,
            Event::Models(scope.clone(), Err("Offline".into())),
            false,
        );
        assert!(st
            .ai
            .model_unavailable(&st.settings.ai, "failed-model")
            .is_some());
        apply(&mut st, Event::Models(scope, Ok(catalog(false))), false);
        assert!(st
            .ai
            .model_unavailable(&st.settings.ai, "failed-model")
            .is_none());
    }
    #[test]
    fn hackclub_refresh_keeps_missing_selection_disabled_and_preserves_draft() {
        let mut st = state();
        st.settings.ai.provider = lumo_core::ai::Provider::Compatible;
        st.settings.ai.models_url = "https://ai.hackclub.com/proxy/v1/models".into();
        st.ai.selected = "gone".into();
        st.ai.draft = "Unsent draft".into();
        let scope = catalog_key(&st);
        apply(
            &mut st,
            Event::Models(
                scope,
                Ok(Catalog {
                    models: vec![Model::manual("new")],
                    saved_at: 1,
                    from_cache: false,
                    warning: None,
                }),
            ),
            false,
        );
        assert_eq!(st.ai.selected, "gone");
        assert_eq!(st.ai.draft, "Unsent draft");
        assert!(st.ai.model(&st.settings.ai).unavailable.is_some());
        assert!(st
            .ai
            .models
            .iter()
            .any(|m| m.id == "gone" && m.unavailable.is_some()));
        st.ai.selected = "missing-manual".into();
        assert!(st.ai.model(&st.settings.ai).unavailable.is_some());
        assert!(!st
            .cmds
            .iter()
            .any(|c| matches!(c, Command::Ai(Action::Send))));
    }
    #[test]
    fn workspace_shortcuts_focus_local_tools_without_sending() {
        let mut st = state();
        st.cmds.clear();
        assert!(workspace_key(&mut st, 0x4e, false, false).is_none());
        assert!(st.cmds.is_empty());
        assert_eq!(
            workspace_key(&mut st, 0x48, true, false),
            Some(crate::ui::wid("ai.history-filter"))
        );
        assert!(st.ai.history);
        workspace_key(&mut st, 0x4b, true, false);
        assert!(st.ai.model_menu && !st.ai.reasoning_menu);
        workspace_key(&mut st, 0x4b, true, false);
        assert!(!st.ai.model_menu);
        workspace_key(&mut st, 0x4e, true, true);
        assert!(st.cmds.contains(&Command::Ai(Action::New(true))));
        assert!(!st
            .cmds
            .iter()
            .any(|c| matches!(c, Command::Ai(Action::Send | Action::Retry))));
    }
    use super::*;
    #[test]
    fn disabled_ai_does_not_load_history_or_fetch_models() {
        let mut st = AppState::new(Default::default(), Default::default());
        st.settings.ai.models_url = "https://example.org/models".into();
        ensure(&mut st);
        assert!(st.cmds.is_empty());
        assert!(!st.ai.loading);
        st.settings.set_addon("ai", true);
        ensure(&mut st);
        assert!(st.cmds.contains(&Command::Ai(Action::Load)));
        assert!(st.cmds.contains(&Command::Ai(Action::EnsureModels)));
    }
    #[test]
    fn image_paste_targets_the_prompt_and_preserves_text_paste_elsewhere() {
        let mut st = state();
        st.ai.draft = "Keep this draft".into();
        st.ai.model_menu = true;
        assert!(!paste_attachment(&mut st, false, || panic!(
            "search fields never inspect clipboard images"
        )));
        assert!(!paste_attachment(&mut st, true, || false));
        assert!(st.cmds.is_empty());
        assert!(paste_attachment(&mut st, true, || true));
        assert_eq!(st.cmds, [Command::Ai(Action::PasteAttachment)]);
        assert!(!st.ai.model_menu);
        assert_eq!(st.ai.draft, "Keep this draft");
        st.ai.attaching = true;
        apply(
            &mut st,
            Event::Attached {
                result: Ok(vec![Attachment {
                    name: "Copied image.png".into(),
                    mime: "image/png".into(),
                    data: "YWJj".into(),
                }]),
                previews: vec![None],
            },
            false,
        );
        assert!(!st.ai.attaching);
        assert_eq!(st.ai.pending.len(), 1);
        assert_eq!(st.ai.draft, "Keep this draft");
    }
    #[test]
    fn model_list_opens_once_and_failed_refresh_preserves_the_usable_list() {
        let mut st = state();
        st.settings.ai.models_url = "https://example.org/models".into();
        let scope = catalog_key(&st);
        ensure(&mut st);
        ensure(&mut st);
        assert_eq!(
            st.cmds
                .iter()
                .filter(|c| **c == Command::Ai(Action::EnsureModels))
                .count(),
            1
        );
        apply(
            &mut st,
            Event::Models(
                scope.clone(),
                Ok(Catalog {
                    models: vec![Model::manual("test/model")],
                    saved_at: 123,
                    from_cache: true,
                    warning: None,
                }),
            ),
            false,
        );
        st.ai.fetching = true;
        apply(
            &mut st,
            Event::Models(scope, Err("Temporary network failure".into())),
            false,
        );
        assert!(!st.ai.fetching);
        assert_eq!(st.ai.models[0].id, "test/model");
        assert_eq!(st.ai.catalog_saved_at, 123);
    }
    fn state() -> AppState {
        let mut st = AppState::new(Default::default(), Default::default());
        st.settings.set_addon("ai", true);
        st.ai.loaded = true;
        st.ai.current = Chat {
            id: 17,
            title: "Test".into(),
            messages: vec![Message {
                role: "user".into(),
                text: "hello".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        st
    }
    #[test]
    fn temporary_chats_and_deleted_chats_stay_out_of_history() {
        let mut st = state();
        st.ai.temporary = true;
        st.ai.snapshot();
        assert!(st.ai.chats.is_empty());
        st.ai.temporary = false;
        st.ai.snapshot();
        assert_eq!(st.ai.chats.len(), 1);
        st.ai.busy = true;
        st.ai.request = 1;
        local(&mut st, &Action::Delete(17));
        apply(
            &mut st,
            Event::Reply {
                request: 1,
                chat: 17,
                result: Ok(Message {
                    role: "assistant".into(),
                    text: "late".into(),
                    ..Default::default()
                }),
                images: vec![],
                unavailable: None,
            },
            true,
        );
        assert!(st.ai.chats.is_empty());
        assert!(st.notice.is_none());
    }
    #[test]
    fn temporary_toggle_preserves_unsent_work_and_never_saves_private_messages() {
        let mut st = state();
        st.ai.draft = "Unsent question".into();
        st.ai.selected = "chosen/model".into();
        st.ai.pending.push(Attachment {
            name: "note.png".into(),
            mime: "image/png".into(),
            data: "YWJj".into(),
        });
        local(&mut st, &Action::ToggleTemporary);
        assert!(st.ai.temporary);
        assert!(st.ai.current.messages.is_empty());
        assert!(st.ai.chats.iter().any(|c| c.id == 17));
        assert_eq!(st.ai.draft, "Unsent question");
        assert_eq!(st.ai.selected, "chosen/model");
        assert_eq!(st.ai.pending.len(), 1);
        let private_id = st.ai.current.id;
        st.ai.current.messages.push(Message {
            role: "user".into(),
            text: "Private conversation".into(),
            ..Default::default()
        });
        local(&mut st, &Action::ToggleTemporary);
        assert!(!st.ai.temporary);
        assert_ne!(st.ai.current.id, private_id);
        assert!(st.ai.current.messages.is_empty());
        assert!(st.ai.chats.iter().all(|c| c.id != private_id));
        assert_eq!(st.ai.draft, "Unsent question");
        assert_eq!(st.ai.selected, "chosen/model");
        assert_eq!(st.ai.pending.len(), 1);
        let empty_id = st.ai.current.id;
        local(&mut st, &Action::ToggleTemporary);
        assert!(st.ai.temporary);
        local(&mut st, &Action::ToggleTemporary);
        assert!(!st.ai.temporary);
        assert_eq!(st.ai.current.id, empty_id);
    }
    #[test]
    fn deleting_history_during_startup_does_not_restore_loaded_chats() {
        let mut st = state();
        st.ai.loaded = false;
        let old = st.ai.current.clone();
        local(&mut st, &Action::DeleteAll);
        apply(
            &mut st,
            Event::Loaded(vec![old], lumo_core::ai::Provider::OpenAI, false),
            false,
        );
        assert!(st.ai.loaded);
        assert!(st.ai.chats.is_empty());
        assert!(st.ai.current.messages.is_empty());
    }
    #[test]
    fn stale_responses_never_replace_current_request() {
        let mut st = state();
        st.ai.busy = true;
        st.ai.request = 2;
        apply(
            &mut st,
            Event::Reply {
                request: 1,
                chat: 17,
                result: Err("old".into()),
                images: vec![],
                unavailable: None,
            },
            true,
        );
        assert!(st.ai.busy);
        assert!(st.ai.error.is_empty());
        assert_eq!(st.ai.current.messages.len(), 1);
    }
    #[test]
    fn cancelled_requests_are_signalled_and_do_not_complete() {
        let mut st = state();
        let c = std::sync::Arc::new(crate::services::http::Cancel::default());
        st.ai.cancel = Some(c.clone());
        st.ai.busy = true;
        st.ai.request = 1;
        local(&mut st, &Action::Stop);
        assert!(c.cancelled());
        assert!(!st.ai.busy);
        assert_eq!(st.ai.generation, Generation::Stopped);
        apply(
            &mut st,
            Event::Reply {
                request: 1,
                chat: 17,
                result: Ok(Message::default()),
                images: vec![],
                unavailable: None,
            },
            true,
        );
        assert!(st.notice.is_none());
        assert_eq!(
            st.ai.generation,
            Generation::Stopped,
            "a late completion cannot replace Stopped"
        );
    }
    #[test]
    fn generation_tracks_the_current_reply_result_and_elapsed_time() {
        let mut st = state();
        st.ai.busy = true;
        st.ai.request = 4;
        st.ai.running_chat = 17;
        st.ai.generation = Generation::Working;
        st.ai.generation_started = 100.0;
        st.now = 112.0;
        apply(
            &mut st,
            Event::Reply {
                request: 4,
                chat: 17,
                result: Err("HTTP 503".into()),
                images: vec![],
                unavailable: None,
            },
            false,
        );
        assert_eq!(st.ai.generation, Generation::Failed);
        assert_eq!(st.ai.generation_finished - st.ai.generation_started, 12.0);
        st.ai.busy = true;
        st.ai.request = 5;
        st.ai.generation = Generation::Working;
        st.ai.generation_started = 120.0;
        st.now = 126.0;
        apply(
            &mut st,
            Event::Reply {
                request: 4,
                chat: 17,
                result: Ok(Message::default()),
                images: vec![],
                unavailable: None,
            },
            false,
        );
        assert_eq!(st.ai.generation, Generation::Working);
        apply(
            &mut st,
            Event::Reply {
                request: 5,
                chat: 17,
                result: Ok(Message {
                    role: "assistant".into(),
                    text: "Ready".into(),
                    ..Default::default()
                }),
                images: vec![],
                unavailable: None,
            },
            false,
        );
        assert_eq!(st.ai.generation, Generation::Ready);
        assert_eq!(st.ai.generation_finished - st.ai.generation_started, 6.0);
        local(&mut st, &Action::New(false));
        assert_eq!(st.ai.generation, Generation::Idle);
    }
    #[test]
    fn unread_reply_targets_its_chat_and_can_be_acknowledged_without_sound() {
        let mut st = state();
        st.ai.busy = true;
        st.ai.request = 9;
        apply(
            &mut st,
            Event::Reply {
                request: 9,
                chat: 17,
                result: Ok(Message {
                    role: "assistant".into(),
                    text: "# Your answer\nDetails".into(),
                    ..Default::default()
                }),
                images: vec![],
                unavailable: None,
            },
            true,
        );
        assert_eq!(st.ai.ready_reply.as_ref().unwrap().chat, 17);
        let notice = st.notice.as_ref().unwrap();
        assert!(notice.is_ai_reply());
        assert_eq!(notice.action, Some(NoticeAction::OpenAiReply(17)));
        assert_eq!(notice.subtitle, "Your answer");
        assert!(!st.cmds.iter().any(|c| matches!(c, Command::Play(_))));
        st.notice = None;
        announce_ready(&mut st);
        assert!(st.notice.as_ref().unwrap().is_ai_reply());
        clear_ready(&mut st);
        assert!(st.ai.ready_reply.is_none() && st.notice.is_none());
    }
}
