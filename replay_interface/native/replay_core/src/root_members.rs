//! The root members of one hour's world body, each behind an `Arc` so hours
//! share every member the recording did not change between them.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value};

#[derive(Clone, Default)]
pub struct RootMembers {
    members: BTreeMap<String, Arc<Value>>,
}

impl RootMembers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_shared(members: impl IntoIterator<Item = (String, Arc<Value>)>) -> Self {
        Self {
            members: members.into_iter().collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.members.get(key).map(Arc::as_ref)
    }

    pub fn get_shared(&self, key: &str) -> Option<&Arc<Value>> {
        self.members.get(key)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.members.contains_key(key)
    }

    /// Copy-on-write: a member another hour still holds is copied first.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.members.get_mut(key).map(Arc::make_mut)
    }

    /// The member as a mutable object, inserted empty when absent.
    pub fn object_mut(&mut self, key: &str) -> Option<&mut Map<String, Value>> {
        self.members
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Value::Object(Map::new())));
        self.get_mut(key).and_then(Value::as_object_mut)
    }

    pub fn insert(&mut self, key: String, value: Value) -> Option<Arc<Value>> {
        self.members.insert(key, Arc::new(value))
    }

    pub fn insert_shared(&mut self, key: String, value: Arc<Value>) -> Option<Arc<Value>> {
        self.members.insert(key, value)
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.members.remove(key).map(Arc::unwrap_or_clone)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.members.iter().map(|(key, value)| (key, value.as_ref()))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.members.keys()
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Whether `key` is the same allocation in both, which is how a member
    /// carried forward untouched from the previous hour is recognised.
    pub fn shares_member(&self, other: &RootMembers, key: &str) -> bool {
        match (self.members.get(key), other.members.get(key)) {
            (Some(mine), Some(theirs)) => Arc::ptr_eq(mine, theirs),
            _ => false,
        }
    }

    /// An owned copy of every member.
    pub fn to_map(&self) -> Map<String, Value> {
        self.members
            .iter()
            .map(|(key, value)| (key.clone(), value.as_ref().clone()))
            .collect()
    }
}

impl PartialEq for RootMembers {
    fn eq(&self, other: &Self) -> bool {
        self.members.len() == other.members.len()
            && self
                .members
                .iter()
                .zip(other.members.iter())
                .all(|((left_key, left), (right_key, right))| {
                    left_key == right_key && (Arc::ptr_eq(left, right) || left == right)
                })
    }
}

impl std::fmt::Debug for RootMembers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.members.iter().map(|(key, value)| (key, value.as_ref())))
            .finish()
    }
}

impl From<Map<String, Value>> for RootMembers {
    fn from(map: Map<String, Value>) -> Self {
        Self {
            members: map
                .into_iter()
                .map(|(key, value)| (key, Arc::new(value)))
                .collect(),
        }
    }
}

impl FromIterator<(String, Value)> for RootMembers {
    fn from_iter<I: IntoIterator<Item = (String, Value)>>(iter: I) -> Self {
        Self {
            members: iter
                .into_iter()
                .map(|(key, value)| (key, Arc::new(value)))
                .collect(),
        }
    }
}

impl std::ops::Index<&str> for RootMembers {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&Value::Null)
    }
}

impl<'a> IntoIterator for &'a RootMembers {
    type Item = (&'a String, &'a Value);
    type IntoIter = Box<dyn Iterator<Item = (&'a String, &'a Value)> + 'a>;
    fn into_iter(self) -> Self::IntoIter {
        Box::new(self.iter())
    }
}

/// A JSON object read by member name: the world's root ([`RootMembers`]) or
/// any nested object (`serde_json::Map`), so one reader serves both.
pub trait MemberLookup {
    fn member(&self, key: &str) -> Option<&Value>;
    fn has_member(&self, key: &str) -> bool {
        self.member(key).is_some()
    }
}

impl MemberLookup for Map<String, Value> {
    fn member(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl MemberLookup for RootMembers {
    fn member(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

/// The world's root as the typed-view builders read it: the shared
/// [`RootMembers`] in the product, a plain `serde_json::Map` in a fixture.
pub trait WorldRoot: MemberLookup {
    fn get(&self, key: &str) -> Option<&Value>;
    fn contains_key(&self, key: &str) -> bool;
    fn iter(&self) -> Box<dyn Iterator<Item = (&String, &Value)> + '_>;
    fn keys(&self) -> Box<dyn Iterator<Item = &String> + '_>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl WorldRoot for Map<String, Value> {
    fn get(&self, key: &str) -> Option<&Value> {
        Map::get(self, key)
    }
    fn contains_key(&self, key: &str) -> bool {
        Map::contains_key(self, key)
    }
    fn iter(&self) -> Box<dyn Iterator<Item = (&String, &Value)> + '_> {
        Box::new(Map::iter(self))
    }
    fn keys(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        Box::new(Map::keys(self))
    }
    fn len(&self) -> usize {
        Map::len(self)
    }
}

impl WorldRoot for RootMembers {
    fn get(&self, key: &str) -> Option<&Value> {
        RootMembers::get(self, key)
    }
    fn contains_key(&self, key: &str) -> bool {
        RootMembers::contains_key(self, key)
    }
    fn iter(&self) -> Box<dyn Iterator<Item = (&String, &Value)> + '_> {
        Box::new(RootMembers::iter(self))
    }
    fn keys(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        Box::new(RootMembers::keys(self))
    }
    fn len(&self) -> usize {
        RootMembers::len(self)
    }
}
