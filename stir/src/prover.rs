use crate::stir_abstractions::{
    Commitment, Domain, FullParameters, Parameters, Proof, Prover, RoundProof, StirProver, Witness,
    WitnessExtended,
};
use crate::utils::{
    dedup, naive_interpolation, poly_fold, poly_quotient, proof_of_work, squeeze_integer,
    stack_evaluations,
};
use ark_crypto_primitives::merkle_tree::Config;
use ark_crypto_primitives::merkle_tree::MerkleTree;
use ark_crypto_primitives::sponge::{Absorb, CryptographicSponge};
use ark_ff::{FftField, PrimeField};
use ark_poly::univariate::DensePolynomial;
use ark_poly::{DenseUVPolynomial, EvaluationDomain, Polynomial};

impl<F, MerkleConfig, FSConfig> StirProver<F, MerkleConfig, FSConfig>
where
    F: FftField + PrimeField + Absorb,
    MerkleConfig: Config<Leaf = Vec<F>>,
    MerkleConfig::InnerDigest: Absorb,
    FSConfig: CryptographicSponge,
    FSConfig::Config: Clone,
{
    // TODO: Rename to better name
    fn round(
        &self,
        sponge: &mut impl CryptographicSponge,
        witness: &WitnessExtended<F, MerkleConfig>,
    ) -> (
        WitnessExtended<F, MerkleConfig>,
        RoundProof<F, MerkleConfig>,
    ) {
        let g_poly = poly_fold(
            &witness.polynomial,
            self.parameters.folding_factor,
            witness.folding_randomness,
        );

        // TODO: For now, I am FFTing
        let g_domain = witness.domain.scale_offset(2);
        let g_evaluations = g_poly
            .evaluate_over_domain_by_ref(g_domain.backing_domain)
            .evals;

        let g_folded_evaluations = stack_evaluations(g_evaluations, self.parameters.folding_factor);
        let g_merkle = MerkleTree::<MerkleConfig>::new(
            &self.parameters.leaf_hash_params,
            &self.parameters.two_to_one_params,
            &g_folded_evaluations,
        )
        .unwrap();
        let g_root = g_merkle.root();
        sponge.absorb(&g_root);

        // Out of domain sample
        let ood_randomness = sponge.squeeze_field_elements(self.parameters.ood_samples);
        let betas = ood_randomness
            .iter()
            .map(|alpha| g_poly.evaluate(alpha))
            .collect();
        sponge.absorb(&betas);

        // Proximity generator
        let comb_randomness: F = sponge.squeeze_field_elements(1)[0];

        // Folding randomness for next round
        let folding_randomness = sponge.squeeze_field_elements(1)[0];

        // Sample the indexes of L^k that we are going to use for querying the previous Merkle tree
        let scaling_factor = witness.domain.size() / self.parameters.folding_factor;
        let num_repetitions = self.parameters.repetitions[witness.num_round];
        let stir_randomness_indexes =
            dedup((0..num_repetitions).map(|_| squeeze_integer(sponge, scaling_factor)));

        let pow_nonce = proof_of_work(sponge, self.parameters.pow_bits[witness.num_round]);

        // Not used
        let _shake_randomness: F = sponge.squeeze_field_elements(1)[0];

        // The verifier queries the previous oracle at the indexes of L^k (reading the
        // corresponding evals)
        let queries_to_prev_ans: Vec<_> = stir_randomness_indexes
            .iter()
            .map(|&index| witness.folded_evals[index].clone())
            .collect();

        let queries_to_prev_proof = witness
            .merkle_tree
            .generate_multi_proof(stir_randomness_indexes.clone())
            .unwrap();
        let queries_to_prev = (queries_to_prev_ans, queries_to_prev_proof);

        // Here, we update the witness
        // First, compute the set of points we are actually going to query at
        let stir_randomness: Vec<_> = stir_randomness_indexes
            .iter()
            .map(|index| {
                witness
                    .domain
                    .scale(self.parameters.folding_factor)
                    .element(*index)
            })
            .collect();

        // Then compute the set we are quotienting by
        let quotient_set: Vec<_> = ood_randomness
            .into_iter()
            .chain(stir_randomness.iter().cloned())
            .collect();

        // TODO: We can probably reuse this in quotient
        let quotient_answers = quotient_set
            .iter()
            .map(|x| (*x, g_poly.evaluate(x)))
            .collect::<Vec<_>>();

        let ans_polynomial = naive_interpolation(&quotient_answers);

        let mut shake_polynomial = DensePolynomial::from_coefficients_vec(vec![]);
        for (x, y) in quotient_answers {
            let num_polynomial = &ans_polynomial - &DensePolynomial::from_coefficients_vec(vec![y]);
            let den_polynomial = DensePolynomial::from_coefficients_vec(vec![-x, F::ONE]);
            shake_polynomial = shake_polynomial + (&num_polynomial / &den_polynomial);
        }

        // The quotient polynomial is then computed
        let quotient_polynomial = poly_quotient(&g_poly, &quotient_set);

        // This is the polynomial 1 + r * x + r^2 * x^2 + ... + r^n * x^n where n = |quotient_set|
        let scaling_polynomial = DensePolynomial::from_coefficients_vec(
            (0..quotient_set.len() + 1)
                .map(|i| comb_randomness.pow([i as u64]))
                .collect(),
        );

        let witness_polynomial = &quotient_polynomial * &scaling_polynomial;

        (
            WitnessExtended {
                domain: g_domain,
                polynomial: witness_polynomial,
                merkle_tree: g_merkle,
                folded_evals: g_folded_evaluations,
                num_round: witness.num_round + 1,
                folding_randomness,
            },
            RoundProof {
                g_root,
                betas,
                queries_to_prev,
                ans_polynomial,
                shake_polynomial,
                pow_nonce,
            },
        )
    }
}

impl<F, MerkleConfig, FSConfig> Prover<F, MerkleConfig, FSConfig>
    for StirProver<F, MerkleConfig, FSConfig>
where
    F: FftField + PrimeField + Absorb,
    MerkleConfig: Config<Leaf = Vec<F>>,
    MerkleConfig::InnerDigest: Absorb,
    FSConfig: CryptographicSponge,
    FSConfig::Config: Clone,
{
    type FullParameter = FullParameters<F, MerkleConfig, FSConfig>;
    type Commitment = Commitment<MerkleConfig>;
    type Witness = Witness<F, MerkleConfig>;
    type Proof = Proof<F, MerkleConfig>;

    fn new(parameters: Parameters<F, MerkleConfig, FSConfig>) -> Self {
        Self::new_full(parameters.into())
    }

    fn new_full(full_parameters: Self::FullParameter) -> Self {
        Self {
            parameters: full_parameters,
        }
    }

    fn commit(
        &self,
        witness_polynomial: DensePolynomial<F>,
    ) -> (Commitment<MerkleConfig>, Witness<F, MerkleConfig>) {
        let domain = Domain::<F>::new(
            self.parameters.starting_degree,
            self.parameters.starting_rate,
        )
        .unwrap();

        let evals = witness_polynomial
            .evaluate_over_domain_by_ref(domain.backing_domain)
            .evals;
        let folded_evals = stack_evaluations(evals, self.parameters.folding_factor);

        let merkle_tree = MerkleTree::<MerkleConfig>::new(
            &self.parameters.leaf_hash_params,
            &self.parameters.two_to_one_params,
            &folded_evals,
        )
        .unwrap();

        let initial_commitment = merkle_tree.root();

        (
            Commitment {
                root: initial_commitment,
            },
            Witness {
                domain,
                polynomial: witness_polynomial,
                merkle_tree,
                folded_evals,
            },
        )
    }

    fn prove(&self, witness: Self::Witness) -> Proof<F, MerkleConfig> {
        assert!(witness.polynomial.degree() < self.parameters.starting_degree);

        let mut sponge = FSConfig::new(&self.parameters.fiat_shamir_config);
        // TODO: Add parameters to FS
        sponge.absorb(&witness.merkle_tree.root());
        let folding_randomness = sponge.squeeze_field_elements(1)[0];

        let mut witness = WitnessExtended {
            domain: witness.domain,
            polynomial: witness.polynomial,
            merkle_tree: witness.merkle_tree,
            folded_evals: witness.folded_evals,
            num_round: 0,
            folding_randomness,
        };

        let mut round_proofs = vec![];
        for _ in 0..self.parameters.num_rounds {
            let (new_witness, round_proof) = self.round(&mut sponge, &witness);
            witness = new_witness;
            round_proofs.push(round_proof);
        }

        let final_polynomial = poly_fold(
            &witness.polynomial,
            self.parameters.folding_factor,
            witness.folding_randomness,
        );

        let final_repetitions = self.parameters.repetitions[self.parameters.num_rounds];
        let scaling_factor = witness.domain.size() / self.parameters.folding_factor;
        let final_randomness_indexes =
            dedup((0..final_repetitions).map(|_| squeeze_integer(&mut sponge, scaling_factor)));

        let queries_to_final_ans: Vec<_> = final_randomness_indexes
            .iter()
            .map(|index| witness.folded_evals[*index].clone())
            .collect();

        let queries_to_final_proof = witness
            .merkle_tree
            .generate_multi_proof(final_randomness_indexes)
            .unwrap();

        let queries_to_final = (queries_to_final_ans, queries_to_final_proof);

        let pow_nonce = proof_of_work(
            &mut sponge,
            self.parameters.pow_bits[self.parameters.num_rounds],
        );

        Proof {
            round_proofs,
            final_polynomial,
            queries_to_final,
            pow_nonce,
        }
    }
}
