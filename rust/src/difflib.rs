//! The part of Python's `difflib.SequenceMatcher` that sourcecheck uses: the
//! Ratcliff/Obershelp longest-match recursion with the default `autojunk`
//! rule and no junk predicate. Ported line for line so the matching blocks,
//! and therefore the counts built on them, are identical to CPython's.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// FxHash, the multiply-rotate hash rustc uses: much cheaper than SipHash for
/// small keys. The maps here are internal and never see untrusted key choice
/// at a scale where flooding matters.
#[derive(Default)]
pub struct FxHasher(u64);

impl Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }
    fn write_u32(&mut self, n: u32) {
        self.write_u64(u64::from(n));
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

pub type FxBuild = BuildHasherDefault<FxHasher>;

/// A run of `size` equal items starting at `a[i]` and `b[j]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub i: usize,
    pub j: usize,
    pub size: usize,
}

/// The `b` side of a matcher: CPython's `b2j` with the autojunk rule
/// applied. It depends on `b` alone, so one index serves many `a` sequences.
pub struct BIndex<'s, T: Eq + std::hash::Hash> {
    b: &'s [T],
    b2j: HashMap<&'s T, Vec<usize>, FxBuild>,
}

impl<'s, T: Eq + std::hash::Hash> BIndex<'s, T> {
    pub fn new(b: &'s [T]) -> Self {
        let mut b2j: HashMap<&T, Vec<usize>, FxBuild> = HashMap::default();
        for (j, item) in b.iter().enumerate() {
            b2j.entry(item).or_default().push(j);
        }
        // autojunk: in a sequence of 200 or more items, an item that fills
        // more than 1% of it (plus one) is "popular" and never starts a match.
        let n = b.len();
        if n >= 200 {
            let ntest = n / 100 + 1;
            b2j.retain(|_, positions| positions.len() <= ntest);
        }
        BIndex { b, b2j }
    }
}

pub struct SequenceMatcher<'s, T: Eq + std::hash::Hash> {
    a: &'s [T],
    b: &'s [T],
    b2j: &'s HashMap<&'s T, Vec<usize>, FxBuild>,
}

impl<'s, T: Eq + std::hash::Hash> SequenceMatcher<'s, T> {
    /// Match `a` against an already indexed `b`.
    pub fn with_index(a: &'s [T], index: &'s BIndex<'s, T>) -> Self {
        SequenceMatcher {
            a,
            b: index.b,
            b2j: &index.b2j,
        }
    }

    /// Longest matching block in `a[alo..ahi]` and `b[blo..bhi]`; ties go to
    /// the earliest start in `a`, then in `b`.
    pub fn find_longest_match(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> Match {
        let (a, b) = (self.a, self.b);
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        // j2len[j] is the length of the match ending at a[i-1], b[j]; CPython
        // keeps it in a dict that is rebuilt per row. Two dense rows plus the
        // list of positions written keep the same values without hashing.
        let mut j2len = vec![0usize; b.len() + 1];
        let mut newj2len = vec![0usize; b.len() + 1];
        let (mut touched, mut newtouched): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        for (i, item) in a.iter().enumerate().take(ahi).skip(alo) {
            if let Some(positions) = self.b2j.get(item) {
                for &j in positions {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = if j == 0 { 0 } else { j2len[j - 1] } + 1;
                    newj2len[j] = k;
                    newtouched.push(j);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            for &j in &touched {
                j2len[j] = 0;
            }
            std::mem::swap(&mut j2len, &mut newj2len);
            std::mem::swap(&mut touched, &mut newtouched);
            newtouched.clear();
        }
        // Extend over items the autojunk rule removed from b2j. There is no
        // junk predicate, so CPython's two junk-extension loops never run.
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi
            && bestj + bestsize < bhi
            && a[besti + bestsize] == b[bestj + bestsize]
        {
            bestsize += 1;
        }
        Match {
            i: besti,
            j: bestj,
            size: bestsize,
        }
    }

    /// Non-overlapping matching blocks in order, adjacent blocks merged,
    /// ending with the sentinel `(len(a), len(b), 0)`.
    pub fn get_matching_blocks(&self) -> Vec<Match> {
        let (la, lb) = (self.a.len(), self.b.len());
        let mut queue = vec![(0, la, 0, lb)];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let m = self.find_longest_match(alo, ahi, blo, bhi);
            if m.size > 0 {
                blocks.push(m);
                if alo < m.i && blo < m.j {
                    queue.push((alo, m.i, blo, m.j));
                }
                if m.i + m.size < ahi && m.j + m.size < bhi {
                    queue.push((m.i + m.size, ahi, m.j + m.size, bhi));
                }
            }
        }
        blocks.sort();
        let mut merged = Vec::new();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        for m in blocks {
            if i1 + k1 == m.i && j1 + k1 == m.j {
                k1 += m.size;
            } else {
                if k1 > 0 {
                    merged.push(Match {
                        i: i1,
                        j: j1,
                        size: k1,
                    });
                }
                (i1, j1, k1) = (m.i, m.j, m.size);
            }
        }
        if k1 > 0 {
            merged.push(Match {
                i: i1,
                j: j1,
                size: k1,
            });
        }
        merged.push(Match {
            i: la,
            j: lb,
            size: 0,
        });
        merged
    }

    /// `(tag, i1, i2, j1, j2)` edit operations turning `a` into `b`.
    pub fn get_opcodes(&self) -> Vec<(Tag, usize, usize, usize, usize)> {
        let (mut i, mut j) = (0, 0);
        let mut answer = Vec::new();
        for m in self.get_matching_blocks() {
            let tag = if i < m.i && j < m.j {
                Some(Tag::Replace)
            } else if i < m.i {
                Some(Tag::Delete)
            } else if j < m.j {
                Some(Tag::Insert)
            } else {
                None
            };
            if let Some(tag) = tag {
                answer.push((tag, i, m.i, j, m.j));
            }
            (i, j) = (m.i + m.size, m.j + m.size);
            if m.size > 0 {
                answer.push((Tag::Equal, m.i, i, m.j, j));
            }
        }
        answer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Replace,
    Delete,
    Insert,
    Equal,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opcodes(a: &str, b: &str) -> Vec<(Tag, usize, usize, usize, usize)> {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        SequenceMatcher::with_index(&a, &BIndex::new(&b)).get_opcodes()
    }

    #[test]
    fn matches_the_cpython_docs_example() {
        // difflib docs: SequenceMatcher(None, "qabxcd", "abycdf").get_opcodes()
        assert_eq!(
            opcodes("qabxcd", "abycdf"),
            vec![
                (Tag::Delete, 0, 1, 0, 0),
                (Tag::Equal, 1, 3, 0, 2),
                (Tag::Replace, 3, 4, 2, 3),
                (Tag::Equal, 4, 6, 3, 5),
                (Tag::Insert, 6, 6, 5, 6),
            ]
        );
    }

    #[test]
    fn longest_match_prefers_earliest_block() {
        // difflib docs: find_longest_match on " abcd" and "abcd abcd" is (0, 4, 5).
        let a: Vec<char> = " abcd".chars().collect();
        let b: Vec<char> = "abcd abcd".chars().collect();
        let index = BIndex::new(&b);
        let m = SequenceMatcher::with_index(&a, &index).find_longest_match(0, 5, 0, 9);
        assert_eq!(
            m,
            Match {
                i: 0,
                j: 4,
                size: 5
            }
        );
    }
}
