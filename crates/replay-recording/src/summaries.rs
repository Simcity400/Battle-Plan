
use replay_columnar::summary::Shape;
use std::io;

pub fn registry() -> io::Result<Vec<(usize, Shape)>> {
    dashboard_declare::summaries()
}
