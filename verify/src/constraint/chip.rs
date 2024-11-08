use crate::constraint::{unflatten_extension, ChipData, ChipMetadata};
use itertools::enumerate;
use p3_field::{ExtensionField, Field, TwoAdicField};
use std::iter::zip;
use std::slice;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DataError {
    #[error("local_variables: incorrect group count (actual: {actual:?}, expected: {expected:?})")]
    NumLocalVariableGroups { actual: usize, expected: usize },
    #[error(
        "local_variables[{group:?}]: incorrect length (actual: {actual:?}, expected: {expected:?})"
    )]
    NumLocalVariables {
        group: usize,
        actual: usize,
        expected: usize,
    },
    #[error(
        "trace_evals: incorrect number of segments (actual: {actual:?}, expected: {expected:?})"
    )]
    NumTraces { actual: usize, expected: usize },
    #[error(
        "trace_evals[segment={segment_index:?}]: incorrect height (actual: {actual:?}, expected: {expected:?})"
    )]
    SegmentHeight {
        segment_index: usize,
        actual: usize,
        expected: usize,
    },
    #[error("trace_evals[segment={segment_index:?}][row={row_index:?}]: incorrect width (actual: {actual:?}, expected: {expected:?})"
    )]
    SegmentRowWidth {
        segment_index: usize,
        row_index: usize,
        actual: usize,
        expected: usize,
    },
    #[error("quotient_evals: incorrect length (actual: {actual:?}, expected: {expected:?})")]
    NumQuotientEvals { actual: usize, expected: usize },
    #[error("trace height smaller than periodic column {column_index:?} (col_len: {col_len:?}, height: {height:?})"
    )]
    MinHeight {
        column_index: usize,
        col_len: usize,
        height: usize,
    },
    #[error("undefined inverse zerofier `{0}` evaluation")]
    UndefinedZerofierEval(usize),
    #[error("invalid quotient ")]
    InvalidQuotient,
}

impl<'a, F: Field, EF: ExtensionField<F>> ChipData<'a, F, EF> {
    pub fn new(
        chip: &'a ChipMetadata<F, EF>,
        local_variables: Vec<Vec<F>>,
        trace_evals: Vec<Vec<Vec<EF>>>,
        quotient_evals: Vec<EF>,
        log_height: usize,
    ) -> Result<Self, DataError> {
        // Check log height is smaller than two-adicity
        // TODO

        // Trace height must be at least the height of each periodic column
        let height = 1 << log_height;
        for (column_index, column) in enumerate(&chip.periodic) {
            if column.len() > height {
                return Err(DataError::MinHeight {
                    column_index,
                    col_len: column.len(),
                    height,
                });
            }
        }

        // Check number of groups of local variables
        if local_variables.len() != chip.num_local_variables.len() {
            return Err(DataError::NumLocalVariableGroups {
                actual: local_variables.len(),
                expected: chip.num_local_variables.len(),
            });
        }

        // Check length of local variables
        for (group, (vars, &expected)) in
            zip(&local_variables, &chip.num_local_variables).enumerate()
        {
            if vars.len() != expected {
                return Err(DataError::NumLocalVariables {
                    group,
                    actual: vars.len(),
                    expected,
                });
            }
        }

        // Check trace dimensions
        if trace_evals.len() != chip.trace_window_dimensions.len() {
            return Err(DataError::NumTraces {
                actual: trace_evals.len(),
                expected: chip.trace_window_dimensions.len(),
            });
        }

        // Check size of trace evaluations
        for (segment_index, (segment_rows, dim)) in
            zip(&trace_evals, &chip.trace_window_dimensions).enumerate()
        {
            if segment_rows.len() != dim.height {
                return Err(DataError::SegmentHeight {
                    segment_index,
                    actual: segment_rows.len(),
                    expected: dim.height,
                });
            }
            for (row_index, row) in enumerate(segment_rows) {
                if row.len() != dim.width {
                    return Err(DataError::SegmentRowWidth {
                        segment_index,
                        row_index,
                        actual: row.len(),
                        expected: dim.width,
                    });
                }
            }
        }

        // Check number of quotient evals
        let num_quotient_evals = chip.num_quotient_evals() * EF::D;
        if quotient_evals.len() != num_quotient_evals {
            return Err(DataError::NumQuotientEvals {
                actual: quotient_evals.len(),
                expected: num_quotient_evals,
            });
        }

        Ok(Self {
            chip,
            local_variables,
            trace_evals,
            quotient_evals,
            log_height,
        })
    }

    pub fn check_quotient(
        &self,
        global_variables: &[Vec<F>],
        zeta: EF,
        alpha: EF,
    ) -> Result<(), DataError>
    where
        F: TwoAdicField,
    {
        let log_n = self.log_height;
        let n = 1 << log_n;
        let g = F::two_adic_generator(log_n);
        // evaluate periodic column at zeta
        let periodic_evals: Vec<EF> = self
            .chip
            .periodic
            .iter()
            .enumerate()
            .map(|(_column_index, _column)| {
                // todo!()
                Ok(EF::zero())
            })
            .collect::<Result<_, _>>()?;

        // Evaluate all nodes
        let mut evals: Vec<EF> = Vec::with_capacity(self.chip.nodes.len());
        for node in &self.chip.nodes {
            evals.push(node.eval(
                &evals,
                global_variables,
                slice::from_ref(&self.local_variables),
                &self.trace_evals,
                &periodic_evals,
            ));
        }

        let inverse_zerofier_evals: Vec<EF> = self
            .chip
            .zerofiers
            .iter()
            .enumerate()
            .map(|(idx, zerofier)| {
                zerofier
                    .eval(zeta, g, n)
                    .and_then(|eval| eval.try_inverse())
                    .ok_or(DataError::UndefinedZerofierEval(idx))
            })
            .collect::<Result<_, _>>()?;

        let quotient = self
            .chip
            .constraints
            .iter()
            .rev()
            .fold(EF::zero(), |acc, constraint| {
                let eval = evals[constraint.node_id]
                    * inverse_zerofier_evals[constraint.zerofier_id.unwrap()];
                acc * alpha + eval
            });

        // eval q(z) = ∑ q_i(z) * z^{ni}
        let zeta_pow_n = zeta.exp_power_of_2(log_n);
        let quotient_expected = self
            .quotient_evals
            .chunks_exact(EF::D)
            .map(unflatten_extension)
            .rev()
            .fold(EF::zero(), |acc, eval| acc * zeta_pow_n + eval);

        if quotient != quotient_expected {
            return Err(DataError::InvalidQuotient);
        }

        Ok(())
    }
}
