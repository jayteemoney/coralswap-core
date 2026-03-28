#![cfg(test)]

use crate::dynamic_fee::{compute_fee_bps, decay_stale_ema, update_volatility};
use crate::errors::PairError;
use crate::storage::FeeState;
use soroban_sdk::{testutils::Ledger, Env};

const SCALE: i128 = 100_000_000_000_000;

fn default_fee_state() -> FeeState {
    FeeState {
        vol_accumulator: 0,
        ema_alpha: SCALE / 20, // 5%
        baseline_fee_bps: 30,
        min_fee_bps: 5,
        max_fee_bps: 100,
        ramp_up_multiplier: 2,
        cooldown_divisor: 2,
        last_fee_update: 0,
        decay_threshold_blocks: 100,
    }
}

// ============================================================================
// update_volatility Tests
// ============================================================================

#[test]
fn test_update_volatility_zero_reserve_returns_error() {
    let env = Env::default();
    let mut fee_state = default_fee_state();

    let result = update_volatility(&env, &mut fee_state, 1000, 100, 0);

    assert_eq!(result, Err(PairError::InvalidInput));
    assert_eq!(fee_state.vol_accumulator, 0);
}

#[test]
fn test_update_volatility_increases_accumulator() {
    let env = Env::default();
    let mut fee_state = default_fee_state();

    let price_delta = 1_000_000_000_000;
    let trade_size = 1_000_000;
    let total_reserve = 10_000_000;

    update_volatility(&env, &mut fee_state, price_delta, trade_size, total_reserve).unwrap();

    assert!(fee_state.vol_accumulator > 0);
}

#[test]
fn test_update_volatility_small_trade_has_less_impact() {
    let env = Env::default();
    let mut fee_state_small = default_fee_state();
    let mut fee_state_large = default_fee_state();

    let price_delta = 1_000_000_000_000;
    let total_reserve = 10_000_000;

    update_volatility(&env, &mut fee_state_small, price_delta, 100_000, total_reserve).unwrap();
    update_volatility(&env, &mut fee_state_large, price_delta, 1_000_000, total_reserve).unwrap();

    assert!(fee_state_large.vol_accumulator > fee_state_small.vol_accumulator);
}

#[test]
fn test_update_volatility_ema_smoothing() {
    let env = Env::default();
    let mut fee_state = default_fee_state();

    let price_delta = 1_000_000_000_000;
    let trade_size = 1_000_000;
    let total_reserve = 10_000_000;

    update_volatility(&env, &mut fee_state, price_delta, trade_size, total_reserve).unwrap();
    let first_value = fee_state.vol_accumulator;

    update_volatility(&env, &mut fee_state, price_delta, trade_size, total_reserve).unwrap();
    let second_value = fee_state.vol_accumulator;

    assert!(second_value > first_value);
    assert!(second_value < first_value * 2);
}

#[test]
fn test_update_volatility_prevents_manipulation_by_tiny_trades() {
    let env = Env::default();
    let mut fee_state = default_fee_state();

    let price_delta = 10_000_000_000_000;
    let tiny_trade = 1;
    let total_reserve = 10_000_000;

    update_volatility(&env, &mut fee_state, price_delta, tiny_trade, total_reserve).unwrap();

    assert!(fee_state.vol_accumulator < price_delta / 1000);
}

// ============================================================================
// compute_fee_bps Tests
// ============================================================================

#[test]
fn test_compute_fee_bps_zero_volatility_returns_baseline() {
    let fee_state = default_fee_state();
    let fee = compute_fee_bps(&fee_state);
    assert_eq!(fee, 30);
}

#[test]
fn test_compute_fee_bps_respects_min_bound() {
    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 1;
    let fee = compute_fee_bps(&fee_state);
    assert!(fee >= fee_state.min_fee_bps);
}

#[test]
fn test_compute_fee_bps_respects_max_bound() {
    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 1_000_000_000_000_000;
    let fee = compute_fee_bps(&fee_state);
    assert!(fee <= fee_state.max_fee_bps);
    assert_eq!(fee, 100);
}

#[test]
fn test_compute_fee_bps_increases_with_volatility() {
    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 1_000_000_000_000;
    let low_fee = compute_fee_bps(&fee_state);

    fee_state.vol_accumulator = 5_000_000_000_000;
    let high_fee = compute_fee_bps(&fee_state);

    assert!(high_fee >= low_fee);
}

// ============================================================================
// decay_stale_ema Tests
// ============================================================================

#[test]
fn test_decay_no_decay_before_threshold() {
    let env = Env::default();
    env.ledger().set_sequence_number(1000);

    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 1_000_000_000_000;
    fee_state.last_fee_update = 500;
    fee_state.decay_threshold_blocks = 1000;

    let initial_vol = fee_state.vol_accumulator;
    let initial_update = fee_state.last_fee_update;

    decay_stale_ema(&env, &mut fee_state);

    assert_eq!(fee_state.vol_accumulator, initial_vol);
    assert_eq!(fee_state.last_fee_update, initial_update);
}

#[test]
fn test_decay_single_period() {
    let env = Env::default();
    env.ledger().set_sequence_number(2001);

    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 1_000_000_000_000;
    fee_state.last_fee_update = 0;
    fee_state.decay_threshold_blocks = 1000;
    fee_state.cooldown_divisor = 2;

    decay_stale_ema(&env, &mut fee_state);

    assert_eq!(fee_state.vol_accumulator, 1_000_000_000_000 / 4);
}

#[test]
fn test_decay_updates_timestamp() {
    let env = Env::default();
    let current_ledger: u32 = 3000;
    env.ledger().set_sequence_number(current_ledger);

    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 1_000_000_000_000;
    fee_state.last_fee_update = 500;
    fee_state.decay_threshold_blocks = 1000;

    decay_stale_ema(&env, &mut fee_state);

    assert_eq!(fee_state.last_fee_update, current_ledger as u64);
}

#[test]
fn test_decay_long_idle_decays_to_zero() {
    let env = Env::default();
    env.ledger().set_sequence_number(50_000);

    let mut fee_state = default_fee_state();
    fee_state.vol_accumulator = 10_000_000_000_000;
    fee_state.last_fee_update = 0;
    fee_state.decay_threshold_blocks = 1000;
    fee_state.cooldown_divisor = 2;

    decay_stale_ema(&env, &mut fee_state);

    assert_eq!(fee_state.vol_accumulator, 0);
}

#[test]
fn test_large_trade_increases_fee() {
    let env = Env::default();
    let mut fee_state = default_fee_state();
    let initial_fee = compute_fee_bps(&fee_state);

    let price_delta = 5_000_000_000_000;
    let trade_size = 2_000_000;
    let total_reserve = 10_000_000;

    update_volatility(&env, &mut fee_state, price_delta, trade_size, total_reserve).unwrap();
    let new_fee = compute_fee_bps(&fee_state);

    assert!(new_fee > initial_fee);
}

#[test]
fn test_fee_stays_within_bounds_under_extreme_conditions() {
    let env = Env::default();
    let mut fee_state = default_fee_state();

    for _ in 0..100 {
        let _ = update_volatility(&env, &mut fee_state, 100_000_000_000_000, 10_000_000, 10_000_000);
    }

    let fee = compute_fee_bps(&fee_state);
    assert!(fee >= fee_state.min_fee_bps);
    assert!(fee <= fee_state.max_fee_bps);
}
