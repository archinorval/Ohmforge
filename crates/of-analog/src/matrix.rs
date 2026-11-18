//! Sparse MNA matrix: pattern built once, values restamped every Newton
//! iteration, factorised with faer's sparse LU while reusing the symbolic
//! analysis (fill-reducing ordering and elimination structure).

use std::collections::BTreeMap;

use faer::c64;
use faer::linalg::solvers::Solve;
use faer::sparse::linalg::solvers::{Lu, SymbolicLu};
use faer::sparse::{SparseColMatRef, SymbolicSparseColMatRef};

/// Handle to one matrix entry. Entries in a ground row or column are
/// [`Slot::NONE`] and stamping them is a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot(u32);

impl Slot {
    /// An entry that touches ground.
    pub const NONE: Slot = Slot(u32::MAX);

    /// Whether this slot refers to a real matrix entry.
    pub fn is_some(self) -> bool {
        self.0 != u32::MAX
    }
}

/// Collects the sparsity pattern while devices are set up.
#[derive(Debug, Default, Clone)]
pub struct Pattern {
    map: BTreeMap<(u32, u32), u32>,
    entries: Vec<(u32, u32)>,
}

impl Pattern {
    /// Slot for row `r`, column `c` in variable numbering (0 = ground).
    pub fn slot(&mut self, r: usize, c: usize) -> Slot {
        if r == 0 || c == 0 {
            return Slot::NONE;
        }
        let key = ((r - 1) as u32, (c - 1) as u32);
        if let Some(&s) = self.map.get(&key) {
            return Slot(s);
        }
        let s = self.entries.len() as u32;
        self.entries.push(key);
        self.map.insert(key, s);
        Slot(s)
    }

    /// Number of distinct entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the pattern has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Why a linear solve failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearError {
    /// The matrix is singular; the index is the first unknown (1-based
    /// variable number) found to be undetermined, when known.
    Singular(Option<usize>),
}

/// Factorisation statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatrixStats {
    /// Numeric LU factorisations performed.
    pub factorisations: u64,
    /// Solves that reused the previous factorisation because the matrix
    /// was unchanged (linear circuits at a fixed step).
    pub reused: u64,
}

/// The assembled system with a cached symbolic factorisation.
pub struct SparseSystem {
    n: usize,
    col_ptr: Vec<usize>,
    row_idx: Vec<usize>,
    /// CSC position of each slot.
    pos: Vec<usize>,
    values: Vec<f64>,
    last_values: Vec<f64>,
    symbolic: Option<SymbolicLu<usize>>,
    lu: Option<Lu<usize, f64>>,
    stats: MatrixStats,
}

impl std::fmt::Debug for SparseSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SparseSystem")
            .field("n", &self.n)
            .field("nnz", &self.row_idx.len())
            .finish_non_exhaustive()
    }
}

impl SparseSystem {
    /// Build the CSC structure for `n` unknowns from `pattern`. Every
    /// diagonal entry is included so gmin can always be added.
    pub fn new(n: usize, pattern: &mut Pattern) -> Self {
        for i in 1..=n {
            pattern.slot(i, i);
        }
        let mut order: Vec<usize> = (0..pattern.entries.len()).collect();
        order.sort_by_key(|&s| (pattern.entries[s].1, pattern.entries[s].0));
        let mut col_ptr = vec![0usize; n + 1];
        let mut row_idx = Vec::with_capacity(order.len());
        let mut pos = vec![0usize; order.len()];
        for (p, &s) in order.iter().enumerate() {
            let (r, c) = pattern.entries[s];
            col_ptr[c as usize + 1] += 1;
            row_idx.push(r as usize);
            pos[s] = p;
        }
        for c in 0..n {
            col_ptr[c + 1] += col_ptr[c];
        }
        let nnz = row_idx.len();
        let symbolic = if n > 0 {
            SymbolicLu::try_new(SymbolicSparseColMatRef::new_checked(n, n, &col_ptr, None, &row_idx)).ok()
        } else {
            None
        };
        Self {
            n,
            col_ptr,
            row_idx,
            pos,
            values: vec![0.0; nnz],
            last_values: Vec::new(),
            symbolic,
            lu: None,
            stats: MatrixStats::default(),
        }
    }

    /// Number of unknowns.
    pub fn dim(&self) -> usize {
        self.n
    }

    /// Number of stored entries.
    pub fn nnz(&self) -> usize {
        self.row_idx.len()
    }

    /// Statistics so far.
    pub fn stats(&self) -> MatrixStats {
        self.stats
    }

    /// Zero all values.
    pub fn clear(&mut self) {
        self.values.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Add `v` to the entry behind `slot`.
    #[inline]
    pub fn add(&mut self, slot: Slot, v: f64) {
        if slot.is_some() {
            self.values[self.pos[slot.0 as usize]] += v;
        }
    }

    /// Current value of an entry (0 for [`Slot::NONE`]).
    pub fn get(&self, slot: Slot) -> f64 {
        if slot.is_some() {
            self.values[self.pos[slot.0 as usize]]
        } else {
            0.0
        }
    }

    /// Assembled values in CSC order (same pattern for every assembly).
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// Solve the complex system `(G + jωC) x = b` in place, where `g` and
    /// `c` are value arrays from [`SparseSystem::values`] on this pattern.
    /// `re`/`im` use variable numbering (index 0 is ground).
    pub fn solve_complex(
        &self,
        g: &[f64],
        c: &[f64],
        omega: f64,
        re: &mut [f64],
        im: &mut [f64],
    ) -> Result<(), LinearError> {
        let n = self.n;
        re[0] = 0.0;
        im[0] = 0.0;
        if n == 0 {
            return Ok(());
        }
        let symbolic = self.symbolic.clone().ok_or(LinearError::Singular(None))?;
        let vals: Vec<c64> = g.iter().zip(c).map(|(&g, &c)| c64::new(g, omega * c)).collect();
        let sym = SymbolicSparseColMatRef::new_checked(n, n, &self.col_ptr, None, &self.row_idx);
        let lu = Lu::<usize, c64>::try_new_with_symbolic(symbolic, SparseColMatRef::new(sym, &vals))
            .map_err(|_| LinearError::Singular(None))?;
        let mut b: Vec<c64> = (1..=n).map(|i| c64::new(re[i], im[i])).collect();
        lu.solve_in_place(faer::MatMut::from_column_major_slice_mut(&mut b, n, 1));
        for (i, v) in b.iter().enumerate() {
            if !(v.re.is_finite() && v.im.is_finite()) {
                return Err(LinearError::Singular(Some(i + 1)));
            }
            re[i + 1] = v.re;
            im[i + 1] = v.im;
        }
        Ok(())
    }

    /// Dense copy (row-major), for tests and diagnostics.
    pub fn to_dense(&self) -> Vec<f64> {
        let n = self.n;
        let mut a = vec![0.0; n * n];
        for c in 0..n {
            for p in self.col_ptr[c]..self.col_ptr[c + 1] {
                a[self.row_idx[p] * n + c] += self.values[p];
            }
        }
        a
    }

    /// Solve `A x = b` in place. `b` uses variable numbering: index 0 is
    /// ground and is ignored (set to 0 on return).
    pub fn solve(&mut self, b: &mut [f64]) -> Result<(), LinearError> {
        let n = self.n;
        b[0] = 0.0;
        if n == 0 {
            return Ok(());
        }
        let reuse = self.lu.is_some() && self.values == self.last_values;
        if reuse {
            self.stats.reused += 1;
        } else {
            let symbolic = self.symbolic.clone().ok_or(LinearError::Singular(None))?;
            let sym = SymbolicSparseColMatRef::new_checked(n, n, &self.col_ptr, None, &self.row_idx);
            let mat = SparseColMatRef::new(sym, &self.values);
            match Lu::try_new_with_symbolic(symbolic, mat) {
                Ok(lu) => self.lu = Some(lu),
                Err(_) => {
                    self.lu = None;
                    return Err(LinearError::Singular(self.find_empty_row()));
                }
            }
            self.last_values.clone_from(&self.values);
            self.stats.factorisations += 1;
        }
        let rhs = &mut b[1..=n];
        if let Some(lu) = &self.lu {
            lu.solve_in_place(faer::MatMut::from_column_major_slice_mut(rhs, n, 1));
        }
        if let Some(bad) = rhs.iter().position(|v| !v.is_finite()) {
            self.lu = None;
            self.last_values.clear();
            return Err(LinearError::Singular(Some(bad + 1)));
        }
        Ok(())
    }

    /// A row or column with no non-zero values, if any (cheap diagnosis).
    fn find_empty_row(&self) -> Option<usize> {
        let mut row_nz = vec![false; self.n];
        let mut col_nz = vec![false; self.n];
        for c in 0..self.n {
            for p in self.col_ptr[c]..self.col_ptr[c + 1] {
                if self.values[p] != 0.0 {
                    row_nz[self.row_idx[p]] = true;
                    col_nz[c] = true;
                }
            }
        }
        (0..self.n).find(|&i| !row_nz[i] || !col_nz[i]).map(|i| i + 1)
    }
}

/// Dense LU with partial pivoting (row-major `a`, `n` x `n`), used to
/// cross-check the sparse path in tests.
pub fn solve_dense(a: &mut [f64], b: &mut [f64], n: usize) -> Result<(), LinearError> {
    for col in 0..n {
        let piv = (col..n)
            .max_by(|&r1, &r2| a[r1 * n + col].abs().total_cmp(&a[r2 * n + col].abs()))
            .unwrap_or(col);
        if a[piv * n + col].abs() < 1e-300 {
            return Err(LinearError::Singular(Some(col + 1)));
        }
        if piv != col {
            for k in 0..n {
                a.swap(piv * n + k, col * n + k);
            }
            b.swap(piv, col);
        }
        for row in col + 1..n {
            let f = a[row * n + col] / a[col * n + col];
            if f != 0.0 {
                for k in col..n {
                    a[row * n + k] -= f * a[col * n + k];
                }
                b[row] -= f * b[col];
            }
        }
    }
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|k| a[row * n + k] * b[k]).sum();
        b[row] = (b[row] - s) / a[row * n + row];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_matches_dense_with_zero_diagonal() {
        // Voltage source branch: zero on the diagonal forces pivoting.
        // Unknowns: v1, v2, i_v.  V=5 between node 1 and ground, 1k 1->2, 2k 2->0.
        let mut p = Pattern::default();
        let s11 = p.slot(1, 1);
        let s12 = p.slot(1, 2);
        let s21 = p.slot(2, 1);
        let s22 = p.slot(2, 2);
        let s13 = p.slot(1, 3);
        let s31 = p.slot(3, 1);
        assert_eq!(p.slot(0, 1), Slot::NONE);
        let mut m = SparseSystem::new(3, &mut p);
        let g1 = 1e-3;
        let g2 = 0.5e-3;
        m.add(s11, g1);
        m.add(s12, -g1);
        m.add(s21, -g1);
        m.add(s22, g1 + g2);
        m.add(s13, 1.0);
        m.add(s31, 1.0);
        m.add(Slot::NONE, 123.0);
        let mut b = vec![0.0, 0.0, 0.0, 5.0];
        let mut a = m.to_dense();
        let mut bd = b[1..].to_vec();
        m.solve(&mut b).unwrap();
        solve_dense(&mut a, &mut bd, 3).unwrap();
        for i in 0..3 {
            assert!((b[i + 1] - bd[i]).abs() < 1e-12);
        }
        assert!((b[1] - 5.0).abs() < 1e-12);
        assert!((b[2] - 5.0 * 2.0 / 3.0).abs() < 1e-12);
        assert!((b[3] + 5.0 / 3000.0).abs() < 1e-15);
        // Same values again: the factorisation is reused.
        let mut b2 = vec![0.0, 0.0, 0.0, 5.0];
        m.solve(&mut b2).unwrap();
        assert_eq!(
            m.stats(),
            MatrixStats {
                factorisations: 1,
                reused: 1
            }
        );
    }

    #[test]
    fn singular_matrix_is_reported() {
        let mut p = Pattern::default();
        let s11 = p.slot(1, 1);
        let mut m = SparseSystem::new(2, &mut p);
        m.add(s11, 1.0);
        let mut b = vec![0.0, 1.0, 1.0];
        assert!(matches!(m.solve(&mut b), Err(LinearError::Singular(Some(2)))));
    }
}
