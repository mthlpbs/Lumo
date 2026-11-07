//! Short-lived workers only: an explicit send, model refresh, attachment or save.
use super::{
    http::{Client, Method, MtaGuard},
    secret,
};
use crate::{
    ai::{self, Action, Event},
    events::{AppEvent, EventSink, SendHwnd},
    state::AppState,
};
use lumo_core::ai::{self as model, Attachment, Chat, Config, Message, Provider};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};

fn key_path(c: &Config) -> PathBuf {
    crate::paths::cache_dir()
        .join("ai")
        .join(format!("key-{:?}.bin", c.provider))
}

fn history_path() -> PathBuf {
    crate::paths::data_dir().join("ai-history.bin")
}
fn spawn(sink: EventSink, f: impl FnOnce() -> Event + Send + 'static) {
    let fallback = sink.clone();
    if std::thread::Builder::new()
        .name("lumo-ai-request".into())
        .spawn(move || {
            let event =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
                    Event::WorkerFailed(
                        "The AI operation stopped unexpectedly. Please try again.".into(),
                    )
                });
            sink.send(AppEvent::Ai(event));
        })
        .is_err()
    {
        fallback.send(AppEvent::Ai(Event::WorkerFailed(
            "Couldn't start the AI worker".into(),
        )));
    }
}
fn headers(c: &Config, key: &str) -> Vec<(String, String)> {
    match c.provider {
        Provider::Claude => vec![
            ("x-api-key".into(), key.into()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ],
        Provider::Gemini => vec![("x-goog-api-key".into(), key.into())],
        _ => {
            if key.is_empty() {
                vec![]
            } else {
                vec![("Authorization".into(), format!("Bearer {key}"))]
            }
        }
    }
}
fn fetch(c: &Config, url: &str, body: Option<&Value>, key: &str) -> Result<Value, String> {
    fetch_attempt(c, url, body, key).map_err(|(message, _)| message)
}
// Only idempotent catalog GETs retry. Never repeat a paid generation request.
fn fetch_models_page(c: &Config, url: &str, key: &str) -> Result<Value, String> {
    match fetch_attempt(c, url, None, key) {
        Ok(v) => Ok(v),
        Err((_, true)) => {
            std::thread::sleep(Duration::from_millis(250));
            fetch_attempt(c, url, None, key).map_err(|(e, _)| e)
        }
        Err((e, false)) => Err(e),
    }
}
fn fetch_attempt(
    c: &Config,
    url: &str,
    body: Option<&Value>,
    key: &str,
) -> Result<Value, (String, bool)> {
    model::validate_url(url).map_err(|e| (e, false))?;
    let _com = MtaGuard::new();
    let client = Client::new(concat!("Lumo/", env!("CARGO_PKG_VERSION")))
        .map_err(|e| (e.to_string(), true))?;
    let hs = headers(c, key);
    let hs: Vec<_> = hs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let json = body.map(Value::to_string);
    let r = client
        .send_bounded(
            if body.is_some() {
                Method::Post
            } else {
                Method::Get
            },
            url,
            &hs,
            json.as_deref(),
            Duration::from_secs(if body.is_some() { 150 } else { 20 }),
            if body.is_some() {
                32 * 1024 * 1024
            } else {
                8 * 1024 * 1024
            },
        )
        .map_err(|e| {
            let retry = matches!(
                e,
                super::http::FetchError::Network(_) | super::http::FetchError::Timeout
            );
            (e.to_string(), retry)
        })?;
    let retry = matches!(r.status, 408 | 425 | 500 | 502 | 503 | 504);
    let v: Value = serde_json::from_str(&r.body).map_err(|_| {
        (
            format!("HTTP {}: The endpoint didn't return JSON", r.status),
            retry,
        )
    })?;
    if !(200..300).contains(&r.status) {
        let detail = v["error"]["message"]
            .as_str()
            .or_else(|| v["error"].as_str())
            .unwrap_or("Request refused");
        return Err((
            format!(
                "HTTP {}: {}",
                r.status,
                detail
                    .replace(key, if key.is_empty() { "" } else { "[key hidden]" })
                    .chars()
                    .take(700)
                    .collect::<String>()
            ),
            retry,
        ));
    }
    Ok(v)
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedCatalog<M> {
    version: u32,
    scope: String,
    saved_at: u64,
    models: M,
}
const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;
fn catalog_path(c: &Config) -> PathBuf {
    crate::paths::cache_dir()
        .join("ai")
        .join(format!("models-{:?}.json", c.provider))
}
fn scope_hash(scope: &str) -> String {
    lumo_core::sha256::hex(&lumo_core::sha256::sha256(scope.as_bytes()))
}
fn load_catalog(path: &std::path::Path, scope: &str) -> Option<ai::Catalog> {
    use std::io::Read;
    if std::fs::metadata(path).ok()?.len() > MAX_CATALOG_BYTES {
        return None;
    }
    let mut bytes = vec![];
    std::fs::File::open(path)
        .ok()?
        .take(MAX_CATALOG_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return None;
    }
    let saved: SavedCatalog<Vec<model::Model>> = serde_json::from_slice(&bytes).ok()?;
    if saved.version != 1
        || saved.scope != scope_hash(scope)
        || saved.models.is_empty()
        || saved.models.len() > 5000
    {
        return None;
    }
    Some(ai::Catalog {
        models: saved.models,
        saved_at: saved.saved_at,
        from_cache: true,
        warning: None,
    })
}
fn save_catalog(
    path: &std::path::Path,
    scope: &str,
    models: &[model::Model],
) -> Result<u64, String> {
    if models.is_empty() || models.len() > 5000 {
        return Err("The model list is empty or too large to save".into());
    }
    let saved_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let bytes = serde_json::to_vec(&SavedCatalog {
        version: 1,
        scope: scope_hash(scope),
        saved_at,
        models,
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return Err("The model list is too large to save".into());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    lumo_core::store::write_bytes_atomic(path, &bytes).map_err(|e| e.to_string())?;
    Ok(saved_at)
}
pub fn run(st: &mut AppState, action: Action, owner: SendHwnd, sink: EventSink) {
    if ai::local(st, &action) {
        return;
    }
    let config = st.settings.ai.clone();
    let scope = ai::catalog_key(st);
    let keyfile = key_path(&config);
    match action {
        Action::Previews => {
            let chat = st.ai.current.clone();
            spawn(sink, move || {
                let mut images = vec![];
                for (mi, m) in chat.messages.iter().enumerate() {
                    if m.role != "assistant" {
                        continue;
                    }
                    for (ai, a) in m.attachments.iter().enumerate() {
                        if images.len() >= 16 {
                            break;
                        }
                        if let Some(image) = model::unbase64(&a.data)
                            .and_then(|b| super::imaging::decode_bytes(&b, 512).ok())
                        {
                            images.push((mi, ai, image));
                        }
                    }
                }
                Event::Previews(chat.id, images)
            });
        }
        Action::CheckKey => {
            spawn(sink, move || {
                Event::KeyChecked(config.provider, secret::load(&keyfile).is_some())
            });
        }
        Action::Load => {
            spawn(sink, move || {
                let path = history_path();
                let chats = if std::fs::metadata(&path).is_ok_and(|m| m.len() <= 40 * 1024 * 1024) {
                    secret::load(&path)
                        .and_then(|s| serde_json::from_str::<Vec<Chat>>(&s).ok())
                        .unwrap_or_default()
                        .into_iter()
                        .take(40)
                        .collect()
                } else {
                    vec![]
                };
                Event::Loaded(chats, config.provider, secret::load(&keyfile).is_some())
            });
        }
        Action::SaveKey | Action::ClearKey => {
            if st.ai.key_saving {
                return;
            }
            let clear = action == Action::ClearKey;
            let key = if clear {
                String::new()
            } else {
                std::mem::take(&mut st.ai.key_draft).trim().to_owned()
            };
            if !clear && key.is_empty() {
                return;
            }
            st.ai.key_saving = true;
            spawn(sink, move || {
                let r = if clear {
                    secret::delete(&keyfile)
                } else {
                    std::fs::create_dir_all(keyfile.parent().unwrap())
                        .and_then(|_| secret::save(&keyfile, &key))
                };
                Event::KeySaved(config.provider, !clear, r.map_err(|e| e.to_string()))
            });
        }
        Action::EnsureModels | Action::FetchModels => {
            if st.ai.fetching || st.ai.key_saving {
                if st.ai.key_saving {
                    st.ai.auto_fetch_key.clear();
                }
                return;
            }
            let refresh = action == Action::FetchModels;
            let same = st.ai.catalog_key == scope && !st.ai.models.is_empty();
            if !refresh && same && (!config.save_models || st.ai.catalog_saved_at > 0) {
                return;
            }
            let prior = (!refresh && same).then(|| st.ai.models.clone());
            let url = config.models().to_owned();
            if let Err(e) = model::validate_url(&url) {
                st.ai.error = e;
                return;
            }
            st.ai.fetching = true;
            spawn(sink, move || {
                let cache = catalog_path(&config);
                if !refresh && config.save_models && prior.is_none() {
                    if let Some(catalog) = load_catalog(&cache, &scope) {
                        return Event::Models(scope, Ok(catalog));
                    }
                }
                let key = secret::load(&keyfile).unwrap_or_default();
                let r = (|| {
                    let from_memory = prior.is_some();
                    let mut all = prior.unwrap_or_default();
                    let mut next = url.clone();
                    for _ in 0..8 {
                        if from_memory {
                            break;
                        }
                        let v = fetch_models_page(&config, &next, &key)?;
                        all.extend(model::parse_models(&v)?);
                        if all.len() >= 5000 {
                            break;
                        }
                        let token = v["nextPageToken"]
                            .as_str()
                            .map(|s| ("pageToken", s))
                            .or_else(|| {
                                if v["has_more"] == true {
                                    v["last_id"].as_str().map(|s| ("after_id", s))
                                } else {
                                    None
                                }
                            });
                        if let Some((name, t)) = token {
                            next = lumo_core::urlenc::with_query(&url, &[(name, t)]);
                        } else {
                            break;
                        }
                    }
                    for m in &mut all {
                        m.reasoning.resolve(&m.id, config.provider);
                        if config.provider != Provider::Compatible {
                            m.provider = match config.provider {
                                Provider::OpenAI => "OpenAI",
                                Provider::Claude => "Anthropic",
                                Provider::Gemini => "Google",
                                _ => "Custom",
                            }
                            .into();
                        }
                    }
                    all.sort_by(|a, b| {
                        (&a.provider, a.kind, a.cost_group(), &a.name).cmp(&(
                            &b.provider,
                            b.kind,
                            b.cost_group(),
                            &b.name,
                        ))
                    });
                    let mut seen = std::collections::HashSet::new();
                    all.retain(|m| seen.insert(m.id.clone()));
                    all.truncate(5000);
                    if all.is_empty() {
                        return Err(
                            "No models were returned. Check the model-list URL and try Refresh."
                                .into(),
                        );
                    }
                    let (mut saved_at, mut warning) = (0, None);
                    if config.save_models {
                        match save_catalog(&cache, &scope, &all) {
                            Ok(t) => saved_at = t,
                            Err(e) => {
                                warning =
                                    Some(format!("Models loaded, but couldn't save the list: {e}"))
                            }
                        }
                    }
                    Ok(ai::Catalog {
                        models: all,
                        saved_at,
                        from_cache: false,
                        warning,
                    })
                })();
                Event::Models(scope, r)
            });
        }
        Action::Send | Action::Retry => {
            if st.ai.busy {
                return;
            }
            let retry = action == Action::Retry;
            let m = st.ai.model(&config);
            if m.id.trim().is_empty() {
                st.ai.error =
                    "Choose a default model in Settings → AI companion, or enter its model ID."
                        .into();
                return;
            }
            let mut messages = st.ai.current.messages.clone();
            if retry {
                while messages.last().is_some_and(|m| m.role == "assistant") {
                    messages.pop();
                }
            } else {
                if st.ai.draft.trim().is_empty() && st.ai.pending.is_empty() {
                    return;
                }
                messages.push(Message {
                    role: "user".into(),
                    text: st.ai.draft.trim().to_owned(),
                    attachments: st.ai.pending.clone(),
                });
            }
            // Bound transmitted context and avoid unexpectedly resending an entire long history.
            if messages.len() > 24 {
                st.ai.error =
                    "This chat has reached 12 turns. Start a new chat to keep requests small."
                        .into();
                return;
            }
            let attachment_bytes: usize = messages
                .iter()
                .flat_map(|m| &m.attachments)
                .map(|a| a.data.len())
                .sum();
            if attachment_bytes > 24 * 1024 * 1024 {
                st.ai.error="This conversation's attachments exceed 18 MB. Start a new chat or remove an attachment.".into();
                return;
            }
            if let Err(e) = model::validate_url(config.completion()) {
                st.ai.error = e;
                return;
            }
            if m.kind == model::Kind::Embedding {
                st.ai.error =
                    "Choose a language or image model to chat. Embeddings are for vector search."
                        .into();
                return;
            }
            if st.ai.current.id == 0 {
                st.ai.current.id = ai::next_id();
            }
            st.ai.current.model = m.id.clone();
            if st.ai.current.title.is_empty() {
                st.ai.current.title = messages
                    .first()
                    .map(|m| m.text.chars().take(65).collect())
                    .unwrap_or_else(|| "New chat".into());
            }
            st.ai.current.messages = messages.clone();
            st.ai.scroll = f32::MAX;
            st.ai.draft.clear();
            st.ai.pending.clear();
            st.ai.close_popovers();
            st.ai.busy = true;
            st.ai.generation = ai::Generation::Working;
            st.ai.generation_started = st.now;
            st.ai.generation_finished = 0.0;
            st.ai.error.clear();
            st.ai.request = st.ai.request.wrapping_add(1);
            let request = st.ai.request;
            let chat = st.ai.current.id;
            st.ai.running_chat = chat;
            st.ai.snapshot();
            st.cmd(crate::commands::Command::Ai(Action::SaveHistory));
            let cancel = std::sync::Arc::new(super::http::Cancel::default());
            st.ai.cancel = Some(cancel.clone());
            spawn(sink, move || {
                let r = super::http::with_cancel(cancel, || {
                    let (url, body) = model::request(&config, &m, &messages)?;
                    drop(messages);
                    let key = secret::load(&keyfile).unwrap_or_default();
                    let v = fetch(&config, &url, Some(&body), &key)?;
                    model::parse_reply(config.provider, &v)
                });
                let images = r
                    .as_ref()
                    .ok()
                    .into_iter()
                    .flat_map(|m| &m.attachments)
                    .filter_map(|a| {
                        model::unbase64(&a.data)
                            .and_then(|b| super::imaging::decode_bytes(&b, 768).ok())
                    })
                    .collect();
                Event::Reply {
                    request,
                    chat,
                    result: r,
                    images,
                }
            });
        }
        Action::SaveHistory => {
            st.ai.snapshot();
            let chats = st.ai.chats.clone();
            use std::sync::{
                atomic::{AtomicU64, Ordering},
                Mutex,
            };
            static REV: AtomicU64 = AtomicU64::new(0);
            static WRITE: Mutex<()> = Mutex::new(());
            let revision = REV.fetch_add(1, Ordering::SeqCst) + 1;
            spawn(sink, move || {
                let _write = WRITE.lock().unwrap_or_else(|e| e.into_inner());
                if REV.load(Ordering::SeqCst) != revision {
                    return Event::Saved(Ok(()));
                }
                let path = history_path();
                let r = (|| {
                    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
                    let s = serde_json::to_string(&chats).map_err(|e| e.to_string())?;
                    if s.len() > 32 * 1024 * 1024 {
                        return Err("History is full. Delete older chats to save this one.".into());
                    }
                    secret::save(&path, &s).map_err(|e| e.to_string())
                })();
                Event::Saved(r)
            });
        }
        Action::Attach(images) => {
            if st.ai.attaching {
                return;
            }
            st.ai.attaching = true;
            spawn(sink, move || {
                let r = unsafe { pick_attachment(owner, images) };
                Event::Attached(r)
            });
        }
        Action::ReadSnip => {
            if st.ai.attaching {
                return;
            }
            st.ai.attaching = true;
            spawn(sink, move || Event::Attached(read_snip(owner)));
        }
        Action::DownloadImage(mi, ai) | Action::CopyImage(mi, ai) => {
            let copy = matches!(action, Action::CopyImage(..));
            if let Some(a) = st
                .ai
                .current
                .messages
                .get(mi)
                .and_then(|m| m.attachments.get(ai))
                .cloned()
            {
                spawn(sink, move || {
                    let r = if copy {
                        copy_image(owner, &a)
                    } else {
                        unsafe { save_image(owner, &a) }
                    };
                    Event::Saved(r)
                });
            }
        }
        _ => {}
    }
}
fn attachment(path: &std::path::Path) -> Result<Attachment, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > 6 * 1024 * 1024 {
        return Err("Choose an image or PDF under 6 MB".into());
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "pdf" => "application/pdf",
        _ => return Err("Choose a PNG, JPEG, WebP, GIF or PDF file".into()),
    };
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    use std::io::Read;
    (&mut file)
        .take(6 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 6 * 1024 * 1024 {
        return Err("The attachment grew beyond 6 MB".into());
    }
    Ok(Attachment {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        mime: mime.into(),
        data: model::base64(&bytes),
    })
}
unsafe fn pick_attachment(owner: SendHwnd, images: bool) -> Result<Vec<Attachment>, String> {
    use windows::core::w;
    use windows::Win32::{System::Com::*, UI::Shell::*};
    CoInitializeEx(None, COINIT_APARTMENTTHREADED)
        .ok()
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let mut dialog = None;
        for attempt in 0..2 {
            let opened = (|| -> windows::core::Result<IFileOpenDialog> {
                let d: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                d.SetTitle(if images {
                    w!("Add images")
                } else {
                    w!("Add PDF documents")
                })?;
                d.SetOptions(
                    FOS_FORCEFILESYSTEM
                        | FOS_FILEMUSTEXIST
                        | FOS_ALLOWMULTISELECT
                        | FOS_NOCHANGEDIR
                        | FOS_STRICTFILETYPES,
                )?;
                d.SetFileTypes(&[windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC {
                    pszName: if images {
                        w!("Images")
                    } else {
                        w!("PDF documents")
                    },
                    pszSpec: if images {
                        w!("*.png;*.jpg;*.jpeg;*.webp;*.gif")
                    } else {
                        w!("*.pdf")
                    },
                }])?;
                d.Show(owner.hwnd())?;
                Ok(d)
            })();
            match opened {
                Ok(d) => {
                    dialog = Some(d);
                    break;
                }
                Err(e)
                    if e.code()
                        == windows::core::HRESULT::from_win32(
                            windows::Win32::Foundation::ERROR_CANCELLED.0,
                        ) =>
                {
                    return Ok(vec![])
                }
                Err(_) if attempt == 0 => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => return Err(format!("Couldn't open the attachment picker: {e}")),
            }
        }
        let d = dialog.ok_or("Couldn't open the attachment picker")?;
        let items = d.GetResults().map_err(|e| e.to_string())?;
        let n = items.GetCount().map_err(|e| e.to_string())?;
        if n > 4 {
            return Err("Attach up to 4 files at once".into());
        }
        let mut out = vec![];
        for i in 0..n {
            let item = items.GetItemAt(i).map_err(|e| e.to_string())?;
            let name = item
                .GetDisplayName(SIGDN_FILESYSPATH)
                .map_err(|e| e.to_string())?;
            let path = name.to_string().map_err(|e| e.to_string());
            CoTaskMemFree(Some(name.0 as _));
            let file = attachment(&PathBuf::from(path?))?;
            if file.mime.starts_with("image/") != images {
                return Err(if images {
                    "Choose an image file"
                } else {
                    "Choose a PDF document"
                }
                .into());
            }
            out.push(file);
        }
        Ok(out)
    })();
    CoUninitialize();
    result
}
unsafe fn save_image(owner: SendHwnd, a: &Attachment) -> Result<(), String> {
    use windows::core::w;
    use windows::Win32::{System::Com::*, UI::Shell::*};
    CoInitializeEx(None, COINIT_APARTMENTTHREADED)
        .ok()
        .map_err(|e| e.to_string())?;
    let r = (|| {
        let d: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| e.to_string())?;
        d.SetTitle(w!("Save generated image"))
            .map_err(|e| e.to_string())?;
        let (filename, extension, pattern) = match a.mime.as_str() {
            "image/png" => (w!("Lumo image.png"), w!("png"), w!("*.png")),
            "image/jpeg" => (w!("Lumo image.jpg"), w!("jpg"), w!("*.jpg")),
            "image/webp" => (w!("Lumo image.webp"), w!("webp"), w!("*.webp")),
            "image/gif" => (w!("Lumo image.gif"), w!("gif"), w!("*.gif")),
            _ => return Err("Unsupported image format".into()),
        };
        d.SetFileName(filename).map_err(|e| e.to_string())?;
        d.SetDefaultExtension(extension)
            .map_err(|e| e.to_string())?;
        d.SetFileTypes(&[windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC {
            pszName: w!("Generated image"),
            pszSpec: pattern,
        }])
        .map_err(|e| e.to_string())?;
        d.SetOptions(FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT | FOS_NOCHANGEDIR)
            .map_err(|e| e.to_string())?;
        if d.Show(owner.hwnd()).is_err() {
            return Ok(());
        }
        let item = d.GetResult().map_err(|e| e.to_string())?;
        let name = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| e.to_string())?;
        let p = name.to_string().map_err(|e| e.to_string());
        CoTaskMemFree(Some(name.0 as _));
        let path = PathBuf::from(p?);
        let bytes = model::unbase64(&a.data).ok_or("The image data is invalid")?;
        lumo_core::store::write_bytes_atomic(&path, &bytes).map_err(|e| e.to_string())
    })();
    CoUninitialize();
    r
}
fn read_snip(owner: SendHwnd) -> Result<Vec<Attachment>, String> {
    let image = clipboard_image(owner)?;
    let rgba = crate::png::pbgra_to_rgba(&image.pixels);
    let bytes = crate::png::encode_rgba(image.width, image.height, &rgba);
    if bytes.len() > 6 * 1024 * 1024 {
        return Err("This image is larger than 6 MB. Choose a smaller image.".into());
    }
    Ok(vec![Attachment {
        name: "Clipboard image.png".into(),
        mime: "image/png".into(),
        data: model::base64(&bytes),
    }])
}
// Checked only for an explicit paste. Never open or monitor the clipboard here.
pub fn clipboard_has_image() -> bool {
    use windows::Win32::System::DataExchange::*;
    unsafe {
        let png = RegisterClipboardFormatW(windows::core::w!("PNG"));
        IsClipboardFormatAvailable(8).is_ok()
            || IsClipboardFormatAvailable(17).is_ok()
            || (png != 0 && IsClipboardFormatAvailable(png).is_ok())
    }
}
fn clipboard_image(owner: SendHwnd) -> Result<crate::gfx::Image, String> {
    use windows::Win32::{
        Foundation::HGLOBAL,
        System::{DataExchange::*, Memory::*},
    };
    unsafe {
        // Snipping Tool can still own the clipboard when Windows posts its update.
        // Retry briefly only for this explicit paste/snipping operation.
        let mut opened = false;
        for _ in 0..8 {
            if OpenClipboard(owner.hwnd()).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if !opened {
            return Err("The clipboard is busy. Try pasting the image again.".into());
        }
        let r = (|| {
            let png = RegisterClipboardFormatW(windows::core::w!("PNG"));
            let format = if png != 0 && IsClipboardFormatAvailable(png).is_ok() {
                png
            } else {
                8
            };
            let h =
                GetClipboardData(format).map_err(|_| "Copy an image first, then paste it here")?;
            let h = HGLOBAL(h.0);
            let size = GlobalSize(h);
            if size > 72 * 1024 * 1024 {
                return Err("Choose a smaller screen snip".into());
            }
            let p = GlobalLock(h);
            if p.is_null() {
                return Err("Couldn't read the screenshot".into());
            }
            let bytes = std::slice::from_raw_parts(p as *const u8, size);
            let r = if format == png {
                super::imaging::decode_bytes(bytes, 4096)
                    .map_err(|_| "Couldn't read the copied image".into())
            } else {
                decode_dib(bytes)
            };
            let _ = GlobalUnlock(h);
            r
        })();
        let _ = CloseClipboard();
        r
    }
}
fn decode_dib(bytes: &[u8]) -> Result<crate::gfx::Image, String> {
    let u32at = |n| {
        bytes
            .get(n..n + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let header = u32at(0).ok_or("Invalid screenshot")? as usize;
    let w = u32at(4).unwrap_or(0) as i32;
    let signed_h = u32at(8).unwrap_or(0) as i32;
    let h = signed_h.unsigned_abs();
    let bpp = bytes
        .get(14..16)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .unwrap_or(0);
    let compression = u32at(16).unwrap_or(9);
    if header < 40
        || w <= 0
        || w > 8192
        || h == 0
        || h > 8192
        || u64::from(h) * (w as u64) > 16_000_000
        || ![24, 32].contains(&bpp)
        || ![0, 3, 6].contains(&compression)
    {
        return Err(
            "The screenshot format isn't supported. Paste a PNG using + → Attach instead.".into(),
        );
    }
    // Windows may synthesize CF_DIB using the standard BGRA bit masks.
    // With a 40-byte header the masks follow it; V4/V5 store them in the header.
    let pixel_offset = if compression == 3 || compression == 6 {
        if bpp != 32
            || u32at(40) != Some(0x00ff0000)
            || u32at(44) != Some(0x0000ff00)
            || u32at(48) != Some(0x000000ff)
        {
            return Err("Unsupported screenshot colour masks. Attach a PNG instead.".into());
        }
        if header == 40 {
            header + if compression == 6 { 16 } else { 12 }
        } else {
            header
        }
    } else {
        header
    };
    let stride = ((w as usize * bpp as usize + 31) / 32) * 4;
    let pixels = bytes
        .get(pixel_offset..)
        .filter(|p| p.len() >= stride * h as usize)
        .ok_or("Incomplete screenshot")?;
    let mut bgra = vec![0; w as usize * h as usize * 4];
    for y in 0..h as usize {
        let sy = if signed_h > 0 { h as usize - 1 - y } else { y };
        for x in 0..w as usize {
            let src = sy * stride + x * (bpp as usize / 8);
            let dst = (y * w as usize + x) * 4;
            bgra[dst..dst + 3].copy_from_slice(&pixels[src..src + 3]);
            bgra[dst + 3] = 255;
        }
    }
    Ok(crate::gfx::ImageData::new(w as u32, h, bgra))
}
fn copy_image(owner: SendHwnd, a: &Attachment) -> Result<(), String> {
    use windows::Win32::{
        Foundation::HANDLE,
        System::{DataExchange::*, Memory::*},
    };
    let bytes = model::unbase64(&a.data).ok_or("Invalid image data")?;
    let image = super::imaging::decode_bytes(&bytes, 4096)?;
    unsafe {
        let size = 40 + image.pixels.len();
        let h = GlobalAlloc(GMEM_MOVEABLE, size).map_err(|e| e.to_string())?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let _ = windows::Win32::Foundation::GlobalFree(h);
            return Err("Couldn't copy the image".into());
        }
        let data = std::slice::from_raw_parts_mut(p, size);
        data.fill(0);
        data[0..4].copy_from_slice(&40u32.to_le_bytes());
        data[4..8].copy_from_slice(&image.width.to_le_bytes());
        data[8..12].copy_from_slice(&(-(image.height as i32)).to_le_bytes());
        data[12..14].copy_from_slice(&1u16.to_le_bytes());
        data[14..16].copy_from_slice(&32u16.to_le_bytes());
        data[40..].copy_from_slice(&image.pixels);
        let _ = GlobalUnlock(h);
        if let Err(e) = OpenClipboard(owner.hwnd()) {
            let _ = windows::Win32::Foundation::GlobalFree(h);
            return Err(e.to_string());
        }
        let r = EmptyClipboard()
            .and_then(|_| SetClipboardData(8, HANDLE(h.0)))
            .map(|_| ())
            .map_err(|e| e.to_string());
        let _ = CloseClipboard();
        if r.is_err() {
            let _ = windows::Win32::Foundation::GlobalFree(h);
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_catalog_roundtrips_and_rejects_changed_endpoint_or_corrupt_data() {
        let path = std::env::temp_dir().join(format!("lumo-catalog-{}.json", ai::next_id()));
        let scope =
            "Compatible|https://example.org/chat|https://example.org/models?key=secret-token";
        let models=model::parse_models(&serde_json::json!({"data":[{"id":"test/reasoner:free","reasoning":{"supported_efforts":["low","high"],"default_effort":"low"},"pricing":{"prompt":"0","completion":"0"}}]})).unwrap();
        let saved = save_catalog(&path, scope, &models).unwrap();
        let loaded = load_catalog(&path, scope).unwrap();
        assert!(loaded.from_cache);
        assert_eq!(loaded.saved_at, saved);
        assert_eq!(loaded.models[0].reasoning.levels, ["low", "high"]);
        assert_eq!(loaded.models[0].free, Some(true));
        assert!(!std::fs::read_to_string(&path)
            .unwrap()
            .contains("secret-token"));
        assert!(load_catalog(&path, "different-provider-or-endpoint").is_none());
        assert!(save_catalog(&path, scope, &[]).is_err());
        assert!(
            load_catalog(&path, scope).is_some(),
            "an empty refresh never overwrites the saved list"
        );
        std::fs::write(&path, b"incomplete json").unwrap();
        assert!(load_catalog(&path, scope).is_none());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn catalog_get_retries_a_transient_first_failure_but_not_authentication_errors() {
        use std::io::{Read, Write};
        for statuses in [vec![503, 200], vec![401]] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap();
            let success = statuses.len() == 2;
            let server = std::thread::spawn(move || {
                for status in statuses {
                    let end = std::time::Instant::now() + Duration::from_secs(5);
                    let mut stream = loop {
                        if let Ok((s, _)) = listener.accept() {
                            break s;
                        }
                        assert!(
                            std::time::Instant::now() < end,
                            "catalog request didn't arrive"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = vec![];
                    let mut bytes = [0; 8192];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = stream.read(&mut bytes).unwrap();
                        assert!(n > 0);
                        request.extend_from_slice(&bytes[..n]);
                    }
                    assert!(String::from_utf8_lossy(&request).starts_with("GET "));
                    let body = if status == 200 {
                        r#"{"data":[{"id":"test/model"}]}"#
                    } else {
                        r#"{"error":{"message":"Try again"}}"#
                    };
                    write!(stream,"HTTP/1.1 {status} Reply\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                }
            });
            let result =
                fetch_models_page(&Config::default(), &format!("http://{addr}/models"), "");
            assert_eq!(result.is_ok(), success, "{result:?}");
            server.join().unwrap();
        }
    }
    #[test]
    fn attachment_reading_accepts_images_and_pdfs_and_rejects_other_files() {
        let root = std::env::temp_dir().join(format!("lumo-attachments-{}", ai::next_id()));
        std::fs::create_dir(&root).unwrap();
        for (ext, allowed) in [
            ("png", true),
            ("JPG", true),
            ("webp", true),
            ("gif", true),
            ("PDF", true),
            ("txt", false),
            ("exe", false),
        ] {
            let path = root.join(format!("sample.{ext}"));
            std::fs::write(&path, b"fixture").unwrap();
            assert_eq!(attachment(&path).is_ok(), allowed);
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    #[ignore = "requires a Windows desktop"]
    fn attachment_picker_opens_first_time_and_cancel_does_not_retry() {
        use windows::Win32::{
            Foundation::*, System::Threading::GetCurrentThreadId, UI::WindowsAndMessaging::*,
        };
        unsafe extern "system" fn find_dialog(hwnd: HWND, lp: LPARAM) -> BOOL {
            let mut title = [0u16; 128];
            let n = GetWindowTextW(hwnd, &mut title);
            let title = String::from_utf16_lossy(&title[..n.max(0) as usize]);
            if title == "Add images" || title == "Add PDF documents" {
                *(lp.0 as *mut Option<HWND>) = Some(hwnd);
                return BOOL(0);
            }
            BOOL(1)
        }
        for images in [true, false] {
            let (tx, rx) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                tx.send(unsafe { GetCurrentThreadId() }).unwrap();
                unsafe { pick_attachment(SendHwnd::new(HWND::default()), images) }
            });
            let thread = rx.recv_timeout(Duration::from_secs(2)).unwrap();
            let end = std::time::Instant::now() + Duration::from_secs(8);
            let mut found = None;
            while found.is_none() && std::time::Instant::now() < end {
                unsafe {
                    let _ = EnumThreadWindows(
                        thread,
                        Some(find_dialog),
                        LPARAM(&mut found as *mut Option<HWND> as isize),
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let hwnd = found.expect("the first request must open the picker");
            unsafe {
                PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)).unwrap();
            }
            assert!(
                worker.join().unwrap().unwrap().is_empty(),
                "cancel is a single completed operation"
            );
        }
    }
    #[test]
    fn native_http_uses_the_selected_provider_protocol_without_external_requests() {
        use std::io::{Read, Write};
        for provider in Provider::ALL {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let end = std::time::Instant::now() + Duration::from_secs(4);
                let mut stream = loop {
                    if let Ok((s, _)) = listener.accept() {
                        break s;
                    }
                    assert!(
                        std::time::Instant::now() < end,
                        "local HTTP client did not connect"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = vec![];
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(i) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..i]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= i + 4 + len {
                            break;
                        }
                    }
                }
                let reply = r#"{"data":[{"id":"test/model"}]}"#;
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",reply.len(),reply).unwrap();
                String::from_utf8(request).unwrap()
            });
            let c = Config {
                provider,
                completion_url: format!("http://{address}/chat"),
                ..Default::default()
            };
            let body = serde_json::json!({"model":"test/model","messages":[{"role":"user","content":"test"}]});
            let reply = fetch(&c, &c.completion_url, Some(&body), "test-key").unwrap();
            assert_eq!(reply["data"][0]["id"], "test/model");
            let request = server.join().unwrap().to_lowercase();
            assert!(request.contains(match provider {
                Provider::Claude => "x-api-key: test-key",
                Provider::Gemini => "x-goog-api-key: test-key",
                _ => "authorization: bearer test-key",
            }));
            assert!(request.contains("application/json"));
        }
    }
    #[test]
    fn stop_cancels_a_waiting_native_request_without_a_poll_timer() {
        use std::io::Read;
        use std::sync::{mpsc, Arc};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (ready_tx, ready) = mpsc::channel();
        let (release_tx, release) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let end = std::time::Instant::now() + Duration::from_secs(4);
            let mut stream = loop {
                if let Ok((s, _)) = listener.accept() {
                    break s;
                }
                assert!(std::time::Instant::now() < end);
                std::thread::sleep(Duration::from_millis(5));
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut b = [0u8; 2048];
            let _ = stream.read(&mut b).unwrap();
            ready_tx.send(()).unwrap();
            let _ = release.recv_timeout(Duration::from_secs(3));
        });
        let cancel = Arc::new(super::super::http::Cancel::default());
        let token = cancel.clone();
        let (done_tx, done) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let c = Config {
                provider: Provider::Compatible,
                ..Default::default()
            };
            let result = super::super::http::with_cancel(token, || {
                fetch(
                    &c,
                    &format!("http://{address}/wait"),
                    Some(&serde_json::json!({"prompt":"test"})),
                    "",
                )
            });
            done_tx.send(result).unwrap();
        });
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        cancel.cancel();
        let result = done
            .recv_timeout(Duration::from_secs(1))
            .expect("Stop should wake the worker immediately");
        assert!(result.unwrap_err().to_lowercase().contains("cancel"));
        let _ = release_tx.send(());
        worker.join().unwrap();
        server.join().unwrap();
    }
    #[test]
    fn screenshot_decoder_handles_bottom_up_and_rejects_oversized_data() {
        let mut dib = vec![0u8; 48];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[40..43].copy_from_slice(&[1, 2, 3]);
        dib[44..47].copy_from_slice(&[4, 5, 6]);
        let i = decode_dib(&dib).unwrap();
        assert_eq!(&i.pixels[..4], &[4, 5, 6, 255]);
        let mut masked = vec![0u8; 56];
        masked[..40].copy_from_slice(&dib[..40]);
        masked[8..12].copy_from_slice(&1i32.to_le_bytes());
        masked[14..16].copy_from_slice(&32u16.to_le_bytes());
        masked[16..20].copy_from_slice(&3u32.to_le_bytes());
        for (offset, value) in [(40, 0x00ff0000u32), (44, 0x0000ff00), (48, 0x000000ff)] {
            masked[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        masked[52..56].copy_from_slice(&[5, 6, 7, 0]);
        assert_eq!(decode_dib(&masked).unwrap().pixels, vec![5, 6, 7, 255]);
        dib[4..8].copy_from_slice(&100000u32.to_le_bytes());
        assert!(decode_dib(&dib).is_err());
    }
}
