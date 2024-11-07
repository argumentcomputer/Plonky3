use crate::constraint::node::{Node, NodesInfo};
use crate::constraint::zerofier::ZerofierExpression;
use p3_field::{ExtensionField, Field, TwoAdicField};
use p3_matrix::Dimensions;
use std::iter::zip;
use std::marker::PhantomData;
use std::slice;

pub mod node;
mod raw_metadata;
pub mod zerofier;

pub type VariableGroupInfo = Vec<usize>;

struct Expression {
    node_id: usize,
    zerofier_id: Option<usize>,
}

pub struct MachineMetadata<F: Field, EF: ExtensionField<F>> {
    num_global_variables: VariableGroupInfo,
    chips: Vec<ChipMetadata<F, EF>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

pub struct ChipMetadata<F: Field, EF: ExtensionField<F>> {
    num_local_variables: VariableGroupInfo,
    trace_window_dimensions: Vec<Dimensions>,
    periodic: Vec<Vec<F>>,
    zerofiers: Vec<ZerofierExpression<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
    degrees: Vec<usize>,
    _marker: PhantomData<EF>,
}

impl<F: Field, EF: ExtensionField<F>> ChipMetadata<F, EF> {
    fn max_constraint_degree(&self) -> usize {
        self.constraints
            .iter()
            .map(|constraint| self.degrees[constraint.node_id])
            .max()
            .unwrap_or(0)
    }

    fn num_quotient_evals(&self) -> usize {
        // We pad to at least degree 2, since a quotient argument doesn't make sense with smaller degrees.
        let max_constraints_degree = self.max_constraint_degree().max(2);

        // The quotient's actual degree is approximately (max_constraint_degree - 1) n,
        // where subtracting 1 comes from division by the zerofier.
        // But we pad it to a power of two so that we can efficiently decompose the quotient.
        (max_constraints_degree - 1).next_power_of_two()
    }

    fn node_info(&self) -> NodesInfo<'_, F, EF> {
        NodesInfo::new_unchecked(&self.nodes)
    }
}

pub struct ChipData<'a, F: Field, EF: ExtensionField<F>> {
    chip: &'a ChipMetadata<F, EF>,
    local_variables: Vec<Vec<F>>,
    // trace_evals[segment][row][column]
    trace_evals: Vec<Vec<Vec<EF>>>,
    quotient_evals: Vec<EF>,
    log_height: usize,
}

impl<'a, F: Field, EF: ExtensionField<F>> ChipData<'a, F, EF> {
    fn new(
        chip: &'a ChipMetadata<F, EF>,
        local_variables: Vec<Vec<F>>,
        trace_evals: Vec<Vec<Vec<EF>>>,
        quotient_evals: Vec<EF>,
        log_height: usize,
    ) -> Result<Self, ()> {
        // Check length of local variables
        if zip(&chip.num_local_variables, &local_variables).any(|(size, vars)| vars.len() != *size)
        {
            return Err(());
        }

        // Check trace dimensions
        if chip.trace_window_dimensions.len() != trace_evals.len() {
            return Err(());
        }

        // Check size of trace evaluations
        for (dim, segment_rows) in zip(&chip.trace_window_dimensions, &trace_evals) {
            if segment_rows.len() != dim.height {
                return Err(());
            }
            if segment_rows.iter().any(|row| row.len() != dim.width) {
                return Err(());
            }
        }

        //
        let num_quotient_evals = chip.num_quotient_evals();
        if quotient_evals.len() != num_quotient_evals * EF::D {
            return Err(());
        }

        Ok(Self {
            chip,
            local_variables,
            trace_evals,
            quotient_evals,
            log_height,
        })
    }

    pub fn check_quotient(&self, global_variables: &[Vec<F>], zeta: EF, alpha: EF) -> Result<(), ()>
    where
        F: TwoAdicField,
    {
        let log_n = self.log_height;
        let n = 1 << log_n;
        let g = F::two_adic_generator(log_n);
        let periodic_evals: Vec<EF> = self
            .chip
            .periodic
            .iter()
            .map(|_col| {
                // todo!()
                EF::zero()
            })
            .collect();

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
            .map(|zerofier| zerofier.eval(zeta, g, n).try_inverse())
            .collect::<Option<Vec<_>>>()
            .ok_or(())?;

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
            return Err(());
        }

        Ok(())
    }
}

fn unflatten_extension<F: Field, EF: ExtensionField<F>>(bases: &[EF]) -> EF {
    assert_eq!(bases.len(), EF::D);
    bases
        .iter()
        .enumerate()
        .map(|(e_i, base_i)| EF::monomial(e_i) * *base_i)
        .sum()
}
