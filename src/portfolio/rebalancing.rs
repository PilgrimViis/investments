use std::collections::{HashSet, HashMap};
use std::cmp::min;

use num_traits::Zero;

use types::Decimal;

use super::asset_allocation::{Portfolio, AssetAllocation, Holding};

// FIXME: implement
pub fn rebalance_portfolio(portfolio: &mut Portfolio) {
    // The first step is bottom-up and calculates strict limits on asset min/max value
    calculate_restrictions(&mut portfolio.assets); // FIXME: Use result

    // The second step is top-down and tries to apply the specified weights and limits calculated in
    // the first step to the actual free assets
    debug!("");
    debug!("Calculating assets target value...");
    calculate_target_value(
        &portfolio.name, &mut portfolio.assets, portfolio.total_value, portfolio.min_trade_volume);

    if false {
        match sell_overbought_assets(&mut portfolio.assets, portfolio.total_value, portfolio.min_trade_volume) {
            SellResult::Ok => (),
            SellResult::Debt(debt) => panic!("Sell failed: {}", debt),
        };
    }
}

fn calculate_restrictions(assets: &mut Vec<AssetAllocation>) -> (Decimal, Option<Decimal>) {
    let mut total_min_value = dec!(0);
    let mut total_max_value = dec!(0);
    let mut all_with_max_value = true;

    for asset in assets {
        let (min_value, max_value) = match &mut asset.holding {
            Holding::Group(assets) => calculate_restrictions(assets),
            Holding::Stock(_) => {
                let min_value = if asset.restrict_selling.unwrap_or(false) {
                    asset.current_value
                } else {
                    dec!(0)
                };

                let max_value = if asset.restrict_buying.unwrap_or(false) {
                    Some(asset.current_value)
                } else {
                    None
                };

                (min_value, max_value)
            },
        };

        asset.min_value = min_value;
        asset.max_value = max_value;

        total_min_value += min_value;

        if let Some(max_value) = max_value {
            total_max_value += max_value;
        } else {
            all_with_max_value = false;
        }
    }

    let total_max_value = if all_with_max_value {
        Some(total_max_value)
    } else {
        None
    };

    (total_min_value, total_max_value)
}

fn calculate_target_value(
    name: &str, assets: &mut Vec<AssetAllocation>, target_total_value: Decimal,
    min_trade_volume: Decimal
) {
    debug!("{name}:", name=name);
    debug!("* Initial target values:");
    for asset in assets.iter_mut() {
        asset.target_value = target_total_value * asset.expected_weight;
        debug!("  * {name} - {value}", name=asset.full_name(), value=asset.target_value);
    }

    let mut balance = dec!(0);
    let mut correctable_holdings = HashSet::new();
    let mut uncorrectable_holdings = HashSet::new();  // FIXME

    for index in 0..assets.len() {
        correctable_holdings.insert(index);
    }

    debug!("* Rebalancing:");

    // First process assets with max value limit to release free cash assets
    for index in &correctable_holdings {
        let asset = &mut assets[*index];

        let max_value = match asset.max_value {
            Some(max_value) => max_value,
            None => continue,
        };

        if asset.target_value > max_value {
            balance += asset.target_value - max_value;
            asset.target_value = max_value;
            asset.buy_blocked = true;

            debug!("  * {name}: buying is blocked at {value}",
                   name=asset.full_name(), value=max_value);
        }
    }

    // Then process assets with min value limit to adapt to restrictions provided by the caller
    for index in &correctable_holdings {
        let asset = &mut assets[*index];
        let min_value = asset.min_value;

        if asset.target_value < min_value {
            balance += asset.target_value - min_value;
            asset.target_value = min_value;
            asset.sell_blocked = true;

            debug!("  * {name}: selling is blocked at {value}",
                   name=asset.full_name(), value=min_value);
        }
    }

    // FIXME: HERE
    let mut sells = Vec::new();
    let mut buys = Vec::new();

    for index in correctable_holdings.clone() {
        let asset = &mut assets[index];

        let difference = asset.target_value - asset.current_value;
        if !difference.is_zero() && difference.abs() < min_trade_volume {
            if difference.is_sign_negative() {
                sells.push((index, -difference));
            } else {
                buys.push((index, difference));
            }

            asset.target_value = asset.current_value;
            balance += difference;
        }
    }

    if !balance.is_zero() {
        if balance.is_sign_positive() {
            buys.sort_by_key(|item| item.1);

            for (index, _) in buys {
                if balance < min_trade_volume {
                    break;
                }

                let asset = &mut assets[index];
                let target_value = asset.current_value + min_trade_volume;

                if let Some(max_value) = asset.max_value {
                    if target_value > max_value {
                        continue;
                    }
                }

                asset.target_value = target_value;
                balance -= min_trade_volume;
            }
        } else {
            sells.sort_by_key(|item| item.1);

            for (index, _) in buys {
                if balance > -min_trade_volume {
                    break;
                }

                let asset = &mut assets[index];
                let target_value = asset.current_value - min_trade_volume;

                if target_value < asset.min_value {
                    if asset.expected_weight.is_zero() && target_value <= dec!(0) {
                        balance += asset.current_value - asset.target_value;
                        asset.target_value = dec!(0);
                    }

                    continue
                }

                asset.target_value = target_value;
                balance += min_trade_volume;
            }
        }
    }

    for index in correctable_holdings.clone() {
        let asset = &mut assets[index];

        let difference = asset.target_value - asset.current_value;

        if difference.is_sign_positive() && balance.is_sign_positive() {
            if let Some(max_value) = asset.max_value {
                let max_volume = max_value - asset.target_value;
                let volume = min(max_volume, balance);
                balance -= volume;
                asset.target_value += volume;
            } else {
                let volume = balance;
                balance -= volume;
                asset.target_value += volume;
            }
        } else if difference.is_sign_negative() && balance.is_sign_negative() {
            let max_volume = asset.target_value - asset.min_value;
            let volume = min(max_volume, -balance);
            balance += volume;
            asset.target_value -= volume;
        }
    }

    info!(">> {}", balance);

    for index in correctable_holdings.union(&uncorrectable_holdings) {
        let asset = &mut assets[*index];
        let asset_name = asset.full_name();

        if let Holding::Group(ref mut holdings) = asset.holding {
            calculate_target_value(&asset_name, holdings, asset.target_value, min_trade_volume);
        }
    }
}

enum SellResult {
    Ok,
    Debt(Decimal),
}

fn sell_overbought_assets(assets: &mut Vec<AssetAllocation>, target_total_value: Decimal, min_trade_volume: Decimal) -> SellResult {
    let mut correctable_holdings = HashSet::new();
    for index in 0..assets.len() {
        correctable_holdings.insert(index);
    }

    let mut force_selling = false;
    let mut uncorrectable_holdings: HashSet<usize> = HashSet::new();

    loop {
        let mut uncorrectable_weight = dec!(0);
        let mut uncorrectable_value = dec!(0);

        for index in &uncorrectable_holdings {
            let asset = &assets[*index];

            uncorrectable_weight += asset.expected_weight;
            uncorrectable_value += asset.target_value;
        }

        let mut correctable_target_total_value = target_total_value - uncorrectable_value;
        let divider = dec!(1) - uncorrectable_weight;
        let mut correctable_debt = dec!(0);

        if correctable_target_total_value.is_sign_negative() {
            correctable_debt = correctable_target_total_value.abs();
            correctable_target_total_value = dec!(0);
        }

        let mut changed = false;

        // FIXME: Sort on force selling
        for index in correctable_holdings.clone() {
            let asset = &mut assets[index];
            let prev_target_value = asset.target_value;

            asset.target_value = correctable_target_total_value * asset.expected_weight / divider;

            match asset.holding {
                Holding::Group(ref mut sub_assets) => {
                    // FIXME: force selling?
                    match sell_overbought_assets(sub_assets, asset.target_value, min_trade_volume) {
                        SellResult::Ok => (),
                        SellResult::Debt(debt) => {
                            correctable_holdings.remove(&index);
                            uncorrectable_holdings.insert(index);

                            assert!(debt > dec!(0));
                            asset.target_value += debt;
                            correctable_debt += debt;
                        },
                    };
                }
                Holding::Stock(ref mut holding) => {
                    if asset.current_value > asset.target_value {
                        if asset.restrict_selling.unwrap_or(false) || asset.current_value < min_trade_volume {
                            let debt = asset.current_value - asset.target_value;
                            assert!(debt > dec!(0));

                            asset.target_value = asset.current_value;
                            correctable_debt += debt;

                            correctable_holdings.remove(&index);
                            uncorrectable_holdings.insert(index);
                        } else if asset.current_value - asset.target_value < min_trade_volume {
                            if force_selling {
                                let target_value = asset.target_value;
                                asset.target_value = asset.current_value - min_trade_volume;

                                let extra_assets = target_value - asset.target_value;
                                assert!(extra_assets >= dec!(0));

                                correctable_debt -= extra_assets;
                                if correctable_debt.is_sign_negative() {
                                    correctable_debt = dec!(0);
                                }

                                // FIXME: HERE
                                if correctable_debt.is_zero() {
                                    break;
                                }
                            } else {
                                let debt = asset.current_value - asset.target_value;
                                assert!(debt > dec!(0));

                                asset.target_value = asset.current_value;
                                correctable_debt += debt;
                            }
                        }

                        // FIXME: HERE
                    }
                },
            };

            changed |= asset.target_value != prev_target_value;
        }

        if correctable_debt.is_zero() {
            return SellResult::Ok;
        }

        if correctable_holdings.is_empty() {
            return SellResult::Debt(correctable_debt);
        }

        if !changed {
            force_selling = true;
        }
    }
}