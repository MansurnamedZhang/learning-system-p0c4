use crate::{DOCUMENT_TITLE, Fixture, Result, fixture::preview_note};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Query, Request, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use learning_core::{
    BlockRef, CompositionRef, ContentDraft, ContentError, ContentRevision, EditReading,
    InsertTarget, PlacementOrder, Principal, ReadingEdit, ReadingMode, ReadingRef,
    SourceProjection, VersionedReadingItem, VersionedReadingProjection,
};
use learning_db::ReadingStore;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const COOKIE_NAME: &str = "kw_reading_preview";
const SESSION_SECONDS: u64 = 1_800;
const NOTE_MAX_BYTES: usize = 4_000;

struct Session {
    actor: Uuid,
    expires: Instant,
}

struct App {
    authority: String,
    origin: String,
    fixture: Fixture,
    owner_code: String,
    visitor_code: String,
    sessions: Mutex<HashMap<Uuid, Session>>,
    reader: PgPool,
    writer: ReadingStore,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SignIn {
    code: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadingQuery {
    mode: ReadingMode,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NoteBase {
    overlay_revision: Uuid,
    reading_revision: Uuid,
    after_placement: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveNote {
    request_id: Uuid,
    base: NoteBase,
    text: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Entry {
    Original {
        title: String,
        text: String,
        block: BlockRef,
        occurrence_path: Vec<Uuid>,
    },
    Personal {
        title: String,
        text: String,
    },
}

#[derive(Serialize)]
struct Reading {
    title: &'static str,
    view: ReadingRef,
    mode: ReadingMode,
    source: Option<CompositionRef>,
    entries: Vec<Entry>,
    unplaced: Vec<Entry>,
    note_base: Option<NoteBase>,
}

fn text(revision: ContentRevision) -> Result<(String, String)> {
    match revision.draft {
        ContentDraft::V1(draft) => Ok((draft.title, draft.payload.text)),
        _ => Err("preview fixture requires v1 text".into()),
    }
}

fn personal(revision: ContentRevision) -> Result<Entry> {
    let (title, text) = text(revision)?;
    Ok(Entry::Personal { title, text })
}

fn reading_dto(
    projected: VersionedReadingProjection,
    mode: ReadingMode,
    note_base: Option<NoteBase>,
) -> Result<Reading> {
    let source = match projected.source {
        SourceProjection::Available { base } => Some(base),
        SourceProjection::Unavailable => None,
    };
    let mut entries = Vec::with_capacity(projected.items.len());
    for item in projected.items {
        entries.push(match item {
            VersionedReadingItem::Original { path, revision } => {
                let block = BlockRef {
                    block_id: revision.block_id,
                    revision_id: revision.revision_id,
                };
                let (title, text) = text(revision)?;
                Entry::Original {
                    title,
                    text,
                    block,
                    occurrence_path: path,
                }
            }
            VersionedReadingItem::Personal { item } => personal(item.revision)?,
            _ => return Err("preview fixture requires a flat document".into()),
        });
    }
    let unplaced = projected
        .unplaced
        .into_iter()
        .map(|item| personal(item.revision))
        .collect::<Result<Vec<_>>>()?;
    Ok(Reading {
        title: if source.is_some() {
            DOCUMENT_TITLE
        } else {
            "个人阅读"
        },
        view: projected.view,
        mode,
        source,
        entries,
        unplaced,
        note_base,
    })
}

fn error(status: StatusCode, code: &'static str) -> Response {
    (status, Json(serde_json::json!({ "error": code }))).into_response()
}

fn session_id(headers: &HeaderMap) -> Option<Uuid> {
    let mut matches = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(name, _)| *name == COOKIE_NAME);
    let (_, token) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Uuid::parse_str(token).ok()
}

fn principal(app: &App, headers: &HeaderMap) -> Option<Principal> {
    let token = session_id(headers)?;
    let sessions = app.sessions.lock().ok()?;
    let session = sessions.get(&token)?;
    (session.expires > Instant::now()).then_some(Principal {
        actor_id: session.actor,
    })
}

fn cookie(token: &str, seconds: u64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={seconds}"
    ))
    .expect("cookie value is generated by the preview")
}

async fn boundary(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok());
    let mut response = if host != Some(app.authority.as_str()) {
        error(StatusCode::MISDIRECTED_REQUEST, "wrong_host")
    } else if !req.method().is_safe() && origin != Some(app.origin.as_str()) {
        error(StatusCode::FORBIDDEN, "wrong_origin")
    } else {
        next.run(req).await
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    response
}

async fn sign_in(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: std::result::Result<Json<SignIn>, JsonRejection>,
) -> Response {
    let Ok(Json(SignIn { code })) = body else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let actor = if bool::from(code.as_bytes().ct_eq(app.owner_code.as_bytes())) {
        app.fixture.owner
    } else if bool::from(code.as_bytes().ct_eq(app.visitor_code.as_bytes())) {
        app.fixture.visitor
    } else {
        return error(StatusCode::UNAUTHORIZED, "invalid_code");
    };
    let Ok(mut sessions) = app.sessions.lock() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "reading_unavailable");
    };
    let now = Instant::now();
    sessions.retain(|_, session| session.expires > now);
    if let Some(previous) = session_id(&headers) {
        sessions.remove(&previous);
    }
    if sessions.len() >= 16 {
        return error(StatusCode::SERVICE_UNAVAILABLE, "session_limit");
    }
    let token = Uuid::new_v4();
    sessions.insert(
        token,
        Session {
            actor,
            expires: now + Duration::from_secs(SESSION_SECONDS),
        },
    );
    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            cookie(&token.to_string(), SESSION_SECONDS),
        )],
    )
        .into_response()
}

async fn sign_out(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Some(token) = session_id(&headers) {
        let Ok(mut sessions) = app.sessions.lock() else {
            return error(StatusCode::SERVICE_UNAVAILABLE, "reading_unavailable");
        };
        sessions.remove(&token);
    }
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cookie("", 0))],
    )
        .into_response()
}

async fn read(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    query: std::result::Result<Query<ReadingQuery>, QueryRejection>,
) -> Response {
    let Some(actor) = principal(&app, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "sign_in_required");
    };
    let Ok(Query(ReadingQuery { mode })) = query else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    match load_reading(&app, actor, mode).await {
        Ok(Some(reading)) => Json(reading).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "reading_unavailable"),
    }
}

async fn load_reading(app: &App, actor: Principal, mode: ReadingMode) -> Result<Option<Reading>> {
    let store = ReadingStore::new(app.reader.clone());
    let Some(state) = store.state(actor, app.fixture.overlay.overlay_id).await? else {
        return Ok(None);
    };
    if state.view.view_id != app.fixture.reading.view_id {
        return Err("unexpected preview reading identity".into());
    }
    let Some(projection) = store
        .read_versioned(actor, state.view.clone(), mode)
        .await?
    else {
        return Ok(None);
    };
    let mut note_base = None;
    if let SourceProjection::Available { base } = &projection.source
        && let Some(layer) = state.editable
        && let Some(group) = layer.groups.iter().find(|group| {
            group.group_id == app.fixture.note_group
                && group.location.placed()
                && &group.location.anchor().base == base
        })
        && let Some(last) = group.placements.last()
    {
        let can_write: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM public.space_grant
                WHERE actor_id = $1 AND space_id = $2 AND can_write)",
        )
        .bind(actor.actor_id)
        .bind(app.fixture.personal_space)
        .fetch_one(&app.reader)
        .await?;
        if can_write {
            note_base = Some(NoteBase {
                overlay_revision: state.overlay.revision_id,
                reading_revision: state.view.revision_id,
                after_placement: last.placement_id,
            });
        }
    }
    reading_dto(projection, mode, note_base).map(Some)
}

async fn save_note(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: std::result::Result<Json<SaveNote>, JsonRejection>,
) -> Response {
    let Some(actor) = principal(&app, &headers) else {
        return error(StatusCode::UNAUTHORIZED, "sign_in_required");
    };
    if actor.actor_id != app.fixture.owner {
        return error(StatusCode::NOT_FOUND, "not_found");
    }
    let Ok(Json(SaveNote {
        request_id,
        base,
        text,
    })) = body
    else {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if request_id.is_nil()
        || text.trim().is_empty()
        || text.len() > NOTE_MAX_BYTES
        || text.contains('\0')
    {
        return error(StatusCode::BAD_REQUEST, "invalid_request");
    }
    let command = EditReading {
        request_id,
        expected_overlay_revision: base.overlay_revision,
        expected_reading_view_revision: base.reading_revision,
        edit: ReadingEdit::InsertNew {
            drafts: vec![preview_note(&text)],
            target: InsertTarget::ExistingGroup {
                group_id: app.fixture.note_group,
                order: PlacementOrder {
                    left_placement_id: Some(base.after_placement),
                    right_placement_id: None,
                },
            },
        },
        reason: "synthetic preview note".into(),
    };
    // Send the submitted base to the store unchanged. It checks the receipt
    // before the current heads so a lost response can be replayed after a save.
    match app
        .writer
        .edit(actor, app.fixture.overlay.overlay_id, command)
        .await
    {
        Ok(saved) => Json(serde_json::json!({
            "request_id": request_id, "revision_id": saved.view.revision_id,
        }))
        .into_response(),
        Err(ContentError::ReadingConflict { .. }) => error(StatusCode::CONFLICT, "stale_revision"),
        Err(ContentError::IdempotencyConflict) => {
            error(StatusCode::CONFLICT, "request_id_conflict")
        }
        Err(ContentError::NotFound) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(ContentError::Invalid(_)) => error(StatusCode::BAD_REQUEST, "invalid_request"),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, "save_unavailable"),
    }
}

pub(crate) fn router(
    reader: PgPool,
    writer: PgPool,
    fixture: Fixture,
    owner_code: String,
    visitor_code: String,
    authority: String,
) -> Result<Router> {
    let is_code = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
    if fixture.owner == fixture.visitor
        || owner_code == visitor_code
        || !is_code(&owner_code)
        || !is_code(&visitor_code)
    {
        return Err("preview requires distinct synthetic subjects and codes".into());
    }
    let state = Arc::new(App {
        origin: format!("http://{authority}"),
        authority,
        fixture,
        owner_code,
        visitor_code,
        sessions: Mutex::default(),
        reader,
        writer: ReadingStore::new(writer),
    });
    Ok(Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../ui/index.html")) }),
        )
        .route(
            "/ui.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../ui/app.js"),
                )
            }),
        )
        .route(
            "/ui.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("../ui/style.css"),
                )
            }),
        )
        .route(
            "/api/session",
            post(sign_in)
                .delete(sign_out)
                .layer(DefaultBodyLimit::max(256)),
        )
        .route("/api/reading", get(read))
        .route(
            "/api/notes",
            post(save_note).layer(DefaultBodyLimit::max(32 * 1024)),
        )
        .fallback(|| async { error(StatusCode::NOT_FOUND, "not_found") })
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state))
}
