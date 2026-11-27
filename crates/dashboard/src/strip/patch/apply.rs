use super::{Operation, Part};
use serde_json::Value;
use std::io;

pub fn apply(input: &Value, patch: &[Operation]) -> io::Result<Value> {
    let mut value = input.clone();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid JSON patch path or operation",
        )
    };
    for operation in patch {
        match operation {
            Operation::Replace(path, replacement) if path.is_empty() => value = replacement.clone(),
            Operation::Replace(path, replacement) => {
                let (last, parent) = path.split_last().ok_or_else(invalid)?;
                match (at(&mut value, parent)?, last) {
                    (Value::Object(object), Part::Key(key)) => {
                        object.insert(key.clone(), replacement.clone());
                    }
                    (Value::Array(array), Part::Index(index)) => {
                        *array.get_mut(*index).ok_or_else(invalid)? = replacement.clone()
                    }
                    _ => return Err(invalid()),
                }
            }
            Operation::Remove(path) => {
                let (last, parent) = path.split_last().ok_or_else(invalid)?;
                let (Value::Object(object), Part::Key(key)) = (at(&mut value, parent)?, last)
                else {
                    return Err(invalid());
                };
                object.remove(key).ok_or_else(invalid)?;
            }
            Operation::Splice(path, index, removed, inserted) => {
                let Value::Array(array) = at(&mut value, path)? else {
                    return Err(invalid());
                };
                let end = index
                    .checked_add(*removed)
                    .filter(|end| *index <= *end && *end <= array.len())
                    .ok_or_else(invalid)?;
                array.splice(*index..end, inserted.iter().cloned());
            }
        }
    }
    Ok(value)
}

fn at<'a>(mut value: &'a mut Value, path: &[Part]) -> io::Result<&'a mut Value> {
    for part in path {
        value = match (value, part) {
            (Value::Object(object), Part::Key(key)) => object.get_mut(key),
            (Value::Array(array), Part::Index(index)) => array.get_mut(*index),
            _ => None,
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid JSON patch path"))?;
    }
    Ok(value)
}
