/// Borrowed native rows used by the decoded-only migration seam.
pub struct Rows<'a, R>(&'a [R]);
impl<R> Clone for Rows<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R> Copy for Rows<'_, R> {}
impl<R: PartialEq> PartialEq for Rows<'_, R> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<R: std::fmt::Debug> std::fmt::Debug for Rows<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0).finish()
    }
}
impl<R> Default for Rows<'_, R> {
    fn default() -> Self {
        Self::empty()
    }
}
impl<'a, R> Rows<'a, R> {
    pub fn empty() -> Self {
        Self(&[])
    }
    pub fn len(self) -> usize {
        self.0.len()
    }
    pub fn is_empty(self) -> bool {
        self.0.is_empty()
    }
    pub fn get(self, index: usize) -> Option<&'a R> {
        self.0.get(index)
    }
    pub fn iter(self) -> std::slice::Iter<'a, R> {
        self.0.iter()
    }
    pub fn is_sorted_by(self, compare: impl FnMut(&R, &R) -> bool) -> bool {
        self.0.is_sorted_by(compare)
    }
    pub fn slice(self, range: std::ops::Range<usize>) -> Self {
        Self(&self.0[range])
    }
    pub fn partition_point(self, predicate: impl FnMut(&R) -> bool) -> usize {
        self.0.partition_point(predicate)
    }
    pub fn binary_search_by_key<K: Ord>(
        self,
        key: &K,
        of: impl FnMut(&R) -> K,
    ) -> Result<usize, usize> {
        self.0.binary_search_by_key(key, of)
    }
}
impl<R> std::ops::Index<usize> for Rows<'_, R> {
    type Output = R;
    fn index(&self, index: usize) -> &R {
        &self.0[index]
    }
}
impl<'a, R> IntoIterator for Rows<'a, R> {
    type Item = &'a R;
    type IntoIter = std::slice::Iter<'a, R>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<'a, R> From<&'a [R]> for Rows<'a, R> {
    fn from(rows: &'a [R]) -> Self {
        Self(rows)
    }
}
impl<'a, R> From<&'a Vec<R>> for Rows<'a, R> {
    fn from(rows: &'a Vec<R>) -> Self {
        Self(rows)
    }
}
impl<'a, R> From<&&'a [R]> for Rows<'a, R> {
    fn from(rows: &&'a [R]) -> Self {
        Self(*rows)
    }
}
impl<'a, R> From<&Rows<'a, R>> for Rows<'a, R> {
    fn from(rows: &Rows<'a, R>) -> Self {
        *rows
    }
}
impl<'a, R, const N: usize> From<&'a [R; N]> for Rows<'a, R> {
    fn from(rows: &'a [R; N]) -> Self {
        Self(rows)
    }
}
