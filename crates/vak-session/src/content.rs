//! Content in a shared record chain belongs to the conversation it came
//! from (plan M7a-b, docs/design/73-data-architecture-and-lifecycle.md
//! §7.3). A row in a chain many conversations share keeps its ids, times
//! and states readable, and holds what a person or a model wrote as one
//! tenant object granted to that conversation's scope, under `sealed`.
//! Destroying the conversation's key leaves the row in place and its
//! content unreadable.

use crate::objects::{ObjectRef, Objects, TenantObjects, conversation_scope};
use crate::types::SessionError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const FIELD: &str = "sealed";

/// Where a row's content is: the object, and the scope that reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    pub scope: String,
    pub object: ObjectRef,
}

/// What reading a row's content gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restored {
    /// The row is whole: it held no sealed content, or it was read back.
    Whole,
    /// The content's conversation was erased; the row has only its ids.
    Erased,
}

fn tenant() -> Result<std::sync::Arc<TenantObjects>, SessionError> {
    TenantObjects::for_tenant(&vak_config::paths::local_tenant_home())
}

fn seal(row: &mut Value, session_id: &str, moved: Map<String, Value>) -> Result<(), SessionError> {
    if moved.is_empty() {
        return Ok(());
    }
    let bytes = serde_json::to_vec(&Value::Object(moved))
        .map_err(|error| SessionError::Objects(error.to_string()))?;
    let scope = conversation_scope(session_id);
    let tenant = tenant()?;
    tenant.create_scope_key(&scope)?;
    let object = tenant.put(&bytes, &scope)?;
    let sealed = serde_json::to_value(Sealed { scope, object })
        .map_err(|error| SessionError::Objects(error.to_string()))?;
    if let Some(fields) = row.as_object_mut() {
        fields.insert(FIELD.into(), sealed);
    }
    Ok(())
}

/// Moves the named `fields` of `row` into an object of `session_id`'s
/// conversation.
pub fn seal_fields(row: &mut Value, session_id: &str, fields: &[&str]) -> Result<(), SessionError> {
    let mut moved = Map::new();
    if let Some(object) = row.as_object_mut() {
        for field in fields {
            if let Some(value) = object.remove(*field) {
                moved.insert((*field).to_string(), value);
            }
        }
    }
    seal(row, session_id, moved)
}

/// Moves every field of `row` but those in `keep` into an object of
/// `session_id`'s conversation.
pub fn seal_except(row: &mut Value, session_id: &str, keep: &[&str]) -> Result<(), SessionError> {
    let mut moved = Map::new();
    if let Some(object) = row.as_object_mut() {
        let names: Vec<String> = object
            .keys()
            .filter(|name| !keep.contains(&name.as_str()))
            .cloned()
            .collect();
        for name in names {
            if let Some(value) = object.remove(&name) {
                moved.insert(name, value);
            }
        }
    }
    seal(row, session_id, moved)
}

/// Puts a row's sealed content back beside its ids. A row whose
/// conversation was erased stays as it is stored and is `Erased`.
pub fn restore(row: &mut Value) -> Result<Restored, SessionError> {
    let Some(sealed) = row.get(FIELD).cloned() else {
        return Ok(Restored::Whole);
    };
    let sealed: Sealed =
        serde_json::from_value(sealed).map_err(|error| SessionError::Objects(error.to_string()))?;
    let tenant = tenant()?;
    if tenant.scope_destroyed(&sealed.scope) {
        return Ok(Restored::Erased);
    }
    let bytes = tenant.get(&sealed.object, &sealed.scope)?;
    let content: Map<String, Value> =
        serde_json::from_slice(&bytes).map_err(|error| SessionError::Objects(error.to_string()))?;
    if let Some(object) = row.as_object_mut() {
        object.remove(FIELD);
        object.extend(content);
    }
    Ok(Restored::Whole)
}
