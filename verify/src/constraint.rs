use crate::constraint::node::NodesInfo;
use crate::constraint::zerofier::ZerofierExpression;
use p3_field::{ExtensionField, Field};
use p3_matrix::Dimensions;
use std::marker::PhantomData;

mod chip;
mod chip_metadata;
mod machine_metadata;
mod node;
mod zerofier;

#[derive(Copy, Clone, Debug)]
enum Node<F: Field> {
    Constant(F),
    /// Base or extension field element from a list of local traces
    Trace {
        segment: usize,
        col_offset: usize,
        row_offset: usize,
        field_type: FieldType,
    },
    /// Base or extension field element local from global (e.g. public values or challenges) or
    /// local (permutation argument hints) variable.
    Var {
        scope: VarScope,
        group: usize,
        offset: usize,
        field_type: FieldType,
    },
    /// Base field elements from local periodic columns
    Periodic {
        column: usize,
    },
    Add {
        lhs_id: usize,
        rhs_id: usize,
    },
    Sub {
        lhs_id: usize,
        rhs_id: usize,
    },
    Mul {
        lhs_id: usize,
        rhs_id: usize,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum VarScope {
    /// Refer to a shared variable (public value, challenge, ...)
    Global,
    /// Refer to a local variable (permutation hint, ...)
    /// When evaluating a trace, `chip_id` must be 0 to refer to itself
    Local { chip_id: usize },
}

#[derive(Copy, Clone, Debug)]
enum FieldType {
    Base,
    Ext,
}

struct Expression {
    node_id: usize,
    zerofier_id: Option<usize>,
}

type VariableGroupInfo = Vec<usize>;
type PeriodicColumn<F> = Vec<F>;

pub struct MachineMetadata<F: Field, EF: ExtensionField<F>> {
    num_global_variables: VariableGroupInfo,
    chips: Vec<ChipMetadata<F, EF>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

pub struct ChipMetadata<F: Field, EF: ExtensionField<F>> {
    num_local_variables: VariableGroupInfo,
    trace_window_dimensions: Vec<Dimensions>,
    periodic: Vec<PeriodicColumn<F>>,
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

fn unflatten_extension<F: Field, EF: ExtensionField<F>>(bases: &[EF]) -> EF {
    assert_eq!(bases.len(), EF::D);
    bases
        .iter()
        .enumerate()
        .map(|(e_i, base_i)| EF::monomial(e_i) * *base_i)
        .sum()
}
