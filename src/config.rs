use std::collections::{HashSet, HashMap};
use std::fs::File;
use std::io::Read;

use chrono::{Duration, Datelike};
use num_traits::FromPrimitive;
use regex::Regex;
use serde::Deserialize;
use serde::de::{Deserializer, Error};

use crate::brokers::Broker;
use crate::core::GenericResult;
use crate::formatting;
use crate::localities::{self, Country};
use crate::taxes::{TaxPaymentDay, TaxRemapping};
use crate::types::{Date, Decimal};
use crate::util::{self, DecimalRestrictions};

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(skip)]
    pub db_path: String,
    #[serde(skip, default = "default_expire_time")]
    pub cache_expire_time: Duration,

    #[serde(default)]
    pub deposits: Vec<DepositConfig>,
    pub notify_deposit_closing_days: Option<u32>,

    #[serde(default)]
    pub portfolios: Vec<PortfolioConfig>,
    pub brokers: Option<BrokersConfig>,

    pub alphavantage: Option<AlphaVantageConfig>,
    pub finnhub: Option<FinnhubConfig>,
    pub twelvedata: Option<TwelveDataConfig>,
}

impl Config {
    #[cfg(test)]
    pub fn mock() -> Config {
        Config {
            db_path: "/mock".to_owned(),
            cache_expire_time: default_expire_time(),

            deposits: Vec::new(),
            notify_deposit_closing_days: None,

            portfolios: Vec::new(),
            brokers: Some(BrokersConfig::mock()),

            alphavantage: None,
            finnhub: None,
            twelvedata: None,
        }
    }

    pub fn get_portfolio(&self, name: &str) -> GenericResult<&PortfolioConfig> {
        for portfolio in &self.portfolios {
            if portfolio.name == name {
                return Ok(portfolio)
            }
        }

        Err!("{:?} portfolio is not defined in the configuration file", name)
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct DepositConfig {
    pub name: String,

    #[serde(deserialize_with = "deserialize_date")]
    pub open_date: Date,
    #[serde(deserialize_with = "deserialize_date")]
    pub close_date: Date,

    #[serde(default)]
    pub currency: Option<String>,
    pub amount: Decimal,
    pub interest: Decimal,
    #[serde(default)]
    pub capitalization: bool,
    #[serde(default, deserialize_with = "deserialize_cash_flows")]
    pub contributions: Vec<(Date, Decimal)>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct PortfolioConfig {
    pub name: String,
    pub broker: Broker,
    pub plan: Option<String>,

    pub statements: String,
    #[serde(default)]
    pub symbol_remapping: HashMap<String, String>,
    #[serde(default)]
    pub instrument_names: HashMap<String, String>,
    #[serde(default)]
    tax_remapping: Vec<TaxRemappingConfig>,

    pub currency: Option<String>,
    pub min_trade_volume: Option<Decimal>,
    pub min_cash_assets: Option<Decimal>,
    pub restrict_buying: Option<bool>,
    pub restrict_selling: Option<bool>,

    #[serde(default)]
    pub merge_performance: HashMap<String, HashSet<String>>,

    #[serde(default)]
    pub assets: Vec<AssetAllocationConfig>,

    #[serde(default, deserialize_with = "deserialize_tax_payment_day")]
    pub tax_payment_day: TaxPaymentDay,

    #[serde(default, deserialize_with = "deserialize_cash_flows")]
    pub tax_deductions: Vec<(Date, Decimal)>,
}

impl PortfolioConfig {
    pub fn get_stock_symbols(&self) -> HashSet<String> {
        let mut symbols = HashSet::new();

        for asset in &self.assets {
            asset.get_stock_symbols(&mut symbols);
        }

        symbols
    }

    pub fn get_tax_country(&self) -> Country {
        localities::russia()
    }

    pub fn get_tax_remapping(&self) -> GenericResult<TaxRemapping> {
        let mut remapping = TaxRemapping::new();

        for config in &self.tax_remapping {
            remapping.add(config.date, &config.description, config.to_date)?;
        }

        Ok(remapping)
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct TaxRemappingConfig {
    #[serde(deserialize_with = "deserialize_date")]
    pub date: Date,
    pub description: String,
    #[serde(deserialize_with = "deserialize_date")]
    pub to_date: Date,
}

#[derive(Deserialize, Debug)]
pub struct AssetAllocationConfig {
    pub name: String,
    pub symbol: Option<String>,

    #[serde(deserialize_with = "deserialize_weight")]
    pub weight: Decimal,
    pub restrict_buying: Option<bool>,
    pub restrict_selling: Option<bool>,

    pub assets: Option<Vec<AssetAllocationConfig>>,
}

impl AssetAllocationConfig {
    fn get_stock_symbols(&self, symbols: &mut HashSet<String>) {
        if let Some(ref symbol) = self.symbol {
            symbols.insert(symbol.to_owned());
        }

        if let Some(ref assets) = self.assets {
            for asset in assets {
                asset.get_stock_symbols(symbols);
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct BrokersConfig {
    pub bcs: Option<BrokerConfig>,
    pub firstrade: Option<BrokerConfig>,
    pub interactive_brokers: Option<BrokerConfig>,
    pub open_broker: Option<BrokerConfig>,
    pub tinkoff: Option<BrokerConfig>,
}

impl BrokersConfig {
    #[cfg(test)]
    pub fn mock() -> BrokersConfig {
        BrokersConfig {
            bcs: Some(BrokerConfig::mock()),
            firstrade: Some(BrokerConfig::mock()),
            interactive_brokers: Some(BrokerConfig::mock()),
            open_broker: Some(BrokerConfig::mock()),
            tinkoff: Some(BrokerConfig::mock()),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct BrokerConfig {
    pub deposit_commissions: HashMap<String, TransactionCommissionSpec>,
}

impl BrokerConfig {
    #[cfg(test)]
    pub fn mock() -> BrokerConfig {
        BrokerConfig {
            deposit_commissions: HashMap::new(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct TransactionCommissionSpec {
    pub fixed_amount: Decimal,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct AlphaVantageConfig {
    pub api_key: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct FinnhubConfig {
    pub token: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct TwelveDataConfig {
    pub token: String,
}

pub fn load_config(path: &str) -> GenericResult<Config> {
    let mut data = Vec::new();
    File::open(path)?.read_to_end(&mut data)?;

    let mut config: Config = serde_yaml::from_slice(&data)?;

    for deposit in &config.deposits {
        if deposit.open_date > deposit.close_date {
            return Err!(
                "Invalid {:?} deposit dates: {} -> {}",
                deposit.name, formatting::format_date(deposit.open_date),
                formatting::format_date(deposit.close_date));
        }

        for &(date, _amount) in &deposit.contributions {
            if date < deposit.open_date || date > deposit.close_date {
                return Err!(
                    "Invalid {:?} deposit contribution date: {}",
                    deposit.name, formatting::format_date(date));
            }
        }
    }

    {
        let mut portfolio_names = HashSet::new();

        for portfolio in &config.portfolios {
            if !portfolio_names.insert(&portfolio.name) {
                return Err!("Duplicate portfolio name: {:?}", portfolio.name);
            }

            if let Some(ref currency) = portfolio.currency {
                match currency.as_str() {
                    "RUB" | "USD" => (),
                    _ => return Err!("Unsupported portfolio currency: {}", currency),
                };
            }

            for (symbol, mapping) in &portfolio.symbol_remapping {
                if portfolio.symbol_remapping.get(mapping).is_some() {
                    return Err!(
                        "Invalid symbol remapping configuration: Recursive {} symbol",
                        symbol);
                }
            }

            let mut symbols_to_merge: HashSet<&String> = HashSet::new();
            for (master_symbol, slave_symbols) in &portfolio.merge_performance {
                if !symbols_to_merge.insert(master_symbol) {
                    return Err!(
                        "Invalid performance merging configuration: Duplicated {} symbol",
                        master_symbol);
                }

                for slave_symbol in slave_symbols {
                    if !symbols_to_merge.insert(slave_symbol) {
                        return Err!(
                            "Invalid performance merging configuration: Duplicated {} symbol",
                            slave_symbol);
                    }
                }
            }
        }
    }

    for portfolio in &mut config.portfolios {
        portfolio.statements = shellexpand::tilde(&portfolio.statements).to_string();
    }

    Ok(config)
}

fn default_expire_time() -> Duration {
    Duration::minutes(1)
}

fn deserialize_tax_payment_day<'de, D>(deserializer: D) -> Result<TaxPaymentDay, D::Error>
    where D: Deserializer<'de>
{
    let tax_payment_day: String = Deserialize::deserialize(deserializer)?;
    if tax_payment_day == "on-close" {
        return Ok(TaxPaymentDay::OnClose);
    }

    Ok(Regex::new(r"^(?P<day>[0-9]+)\.(?P<month>[0-9]+)$").unwrap().captures(&tax_payment_day).and_then(|captures| {
        let day = captures.name("day").unwrap().as_str().parse::<u32>().ok();
        let month = captures.name("month").unwrap().as_str().parse::<u32>().ok();
        let (day, month) = match (day, month) {
            (Some(day), Some(month)) => (day, month),
            _ => return None,
        };

        if Date::from_ymd_opt(util::today().year(), month, day).is_none() || (day, month) == (29, 2) {
            return None;
        }

        Some(TaxPaymentDay::Day {month, day})
    }).ok_or_else(|| D::Error::custom(format!("Invalid tax payment day: {:?}", tax_payment_day)))?)
}

fn deserialize_cash_flows<'de, D>(deserializer: D) -> Result<Vec<(Date, Decimal)>, D::Error>
    where D: Deserializer<'de>
{
    let deserialized: HashMap<String, String> = Deserialize::deserialize(deserializer)?;
    let mut cash_flows = Vec::new();

    for (date, amount) in deserialized {
        let date = util::parse_date(&date, "%d.%m.%Y").map_err(D::Error::custom)?;
        let amount = util::parse_decimal(&amount, DecimalRestrictions::StrictlyPositive).map_err(|_|
            D::Error::custom(format!("Invalid amount: {:?}", amount)))?;

        cash_flows.push((date, amount));
    }

    cash_flows.sort_by_key(|cash_flow| cash_flow.0);

    Ok(cash_flows)
}

fn deserialize_date<'de, D>(deserializer: D) -> Result<Date, D::Error>
    where D: Deserializer<'de>
{
    let date: String = Deserialize::deserialize(deserializer)?;
    Ok(util::parse_date(&date, "%d.%m.%Y").map_err(D::Error::custom)?)
}

fn deserialize_weight<'de, D>(deserializer: D) -> Result<Decimal, D::Error>
    where D: Deserializer<'de>
{
    let weight: String = Deserialize::deserialize(deserializer)?;
    if !weight.ends_with('%') {
        return Err(D::Error::custom(format!("Invalid weight: {}", weight)));
    }

    let weight = match weight[..weight.len() - 1].parse::<u8>().ok() {
        Some(weight) if weight <= 100 => weight,
        _ => return Err(D::Error::custom(format!("Invalid weight: {}", weight))),
    };

    Ok(Decimal::from_u8(weight).unwrap() / dec!(100))
}