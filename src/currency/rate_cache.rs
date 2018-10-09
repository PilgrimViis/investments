use std::str::FromStr;

use chrono::{self, Datelike, Duration};
use diesel::{self, prelude::*};
#[cfg(test)] use tempfile::NamedTempFile;

use core::{GenericResult, GenericError, EmptyResult};
use currency::CurrencyRate;
use db::{self, schema::currency_rates, models};
use types::{Date, Decimal};

pub struct CurrencyRateCache {
    today: Date,
    db: db::Connection,
}

impl CurrencyRateCache {
    pub fn new(connection: db::Connection) -> CurrencyRateCache {
        let today = chrono::Local::today();

        CurrencyRateCache {
            today: Date::from_ymd(today.year(), today.month(), today.day()),
            db: connection,
        }
    }

    #[cfg(test)]
    pub fn new_temporary() -> (NamedTempFile, CurrencyRateCache) {
        let database = NamedTempFile::new().unwrap();
        let connection = db::connect(database.path().to_str().unwrap()).unwrap();
        (database, CurrencyRateCache::new(connection))
    }

    pub fn today(&self) -> Date {
        self.today
    }

    pub fn get(&self, currency: &str, date: Date) -> GenericResult<CurrencyRateCacheResult> {
        if date >= self.today {
            return Err!("An attempt to get price for the future")
        }

        self.db.transaction::<_, GenericError, _>(|| {
            let result = currency_rates::table
                .select(currency_rates::price)
                .filter(currency_rates::currency.eq(currency))
                .filter(currency_rates::date.eq(&date))
                .get_result::<Option<String>>(&self.db).optional()?;

            if let Some(cached_price) = result {
                return Ok(CurrencyRateCacheResult::Exists(match cached_price {
                    Some(price) => Some(Decimal::from_str(&price).map_err(|_| format!(
                        "Got an invalid price from the database: {:?}", price))?),
                    None => None,
                }));
            }

            let year_start = Date::from_ymd(date.year(), 1, 1);
            let year_end = Date::from_ymd(date.year(), 12, 31);

            let last_date = currency_rates::table
                .select(currency_rates::date)
                .filter(currency_rates::currency.eq(currency))
                .filter(currency_rates::date.ge(year_start))
                .filter(currency_rates::date.le(year_end))
                .order(currency_rates::date.desc())
                .limit(1)
                .get_result::<Date>(&self.db).optional()?;

            let start_date = match last_date {
                Some(last_date) => last_date + Duration::days(1),
                None => year_start,
            };

            let end_date = if year_end >= self.today {
                self.today - Duration::days(1)
            } else {
                year_end
            };

            assert!(start_date <= end_date);
            assert!(end_date < self.today);

            Ok(CurrencyRateCacheResult::Missing(start_date, end_date))
        })
    }

    pub fn save(&self, currency: &str, start_date: Date, end_date: Date, mut rates: Vec<CurrencyRate>) -> EmptyResult {
        if start_date > end_date {
            return Err!("Invalid date range: {} - {}", start_date, end_date);
        } else if end_date >= self.today {
            return Err!("An attempt to save currency rates for the future");
        }

        rates.sort_by_key(|rate| rate.date);

        if !rates.is_empty() && (
            rates.first().unwrap().date < start_date ||
            rates.last().unwrap().date > end_date
        ) {
            return Err!("The specified currency rates don't match the specified date range");
        }

        let mut values = Vec::new();
        let fill_gap = |values: &mut Vec<_>, mut from, to| {
            while from < to {
                values.push(models::NewCurrencyRate {
                    currency: currency,
                    date: from,
                    price: None,
                });
                from += Duration::days(1);
            }
        };

        let mut next_date = start_date;

        for rate in &rates {
            fill_gap(&mut values, next_date, rate.date);

            values.push(models::NewCurrencyRate {
                currency: currency,
                date: rate.date,
                price: Some(rate.price.to_string()),
            });
            next_date = rate.date + Duration::days(1);
        }

        fill_gap(&mut values, next_date, end_date + Duration::days(1));

        diesel::replace_into(currency_rates::table)
            .values(&values)
            .execute(&self.db)?;

        Ok(())
    }
}

#[derive(Debug)]
pub enum CurrencyRateCacheResult {
    Exists(Option<Decimal>),
    Missing(Date, Date),
}

#[cfg(test)]
mod tests {
    use bigdecimal::FromPrimitive;
    use super::*;

    #[test]
    fn rate_cache() {
        let currency = "USD";
        let (_database, mut cache) = CurrencyRateCache::new_temporary();

        let today = Date::from_ymd(2018, 2, 9);
        cache.today = today;

        let currency_rates = vec![CurrencyRate {
            date: Date::from_ymd(2018, 2, 4),
            price: Decimal::from_i64(1).unwrap() / 3,
        }, CurrencyRate {
            date: Date::from_ymd(2018, 1, 10),
            price: Decimal::from_i64(1).unwrap() / 7,
        }];

        assert_matches!(
            cache.get(currency, today),
            Err(ref e) if e.to_string() == "An attempt to get price for the future"
        );

        assert_matches!(
            cache.get(currency, currency_rates.first().unwrap().date).unwrap(),
            CurrencyRateCacheResult::Missing(from, to) if (
                from == Date::from_ymd(2018, 1, 1) && to == Date::from_ymd(2018, 2, 8))
        );

        cache.save(currency, Date::from_ymd(2018, 1, 1), Date::from_ymd(2018, 2, 8),
                   currency_rates.clone()).unwrap();

        for currency_rate in &currency_rates {
            assert_matches!(
                cache.get(currency, currency_rate.date).unwrap(),
                CurrencyRateCacheResult::Exists(Some(ref price)) if *price == currency_rate.price
            );
        }

        let mut date = Date::from_ymd(2018, 1, 1);
        while date < cache.today {
            let mut skip = false;

            for currency_rate in &currency_rates {
                if date == currency_rate.date {
                    skip = true;
                    break;
                }
            }

            if !skip {
                let result = cache.get(currency, date).unwrap();
                assert_matches!(result, CurrencyRateCacheResult::Exists(None))
            }

            date += Duration::days(1);
        }

        assert_matches!(
            cache.get(currency, Date::from_ymd(2017, 12, 31)).unwrap(),
            CurrencyRateCacheResult::Missing(from, to) if (
                from == Date::from_ymd(2017, 1, 1) && to == Date::from_ymd(2017, 12, 31))
        );

        cache.today = today + Duration::days(10);

        assert_matches!(
            cache.get(currency, today).unwrap(),
            CurrencyRateCacheResult::Missing(from, to) if (
                from == today && to == cache.today - Duration::days(1))
        );
    }
}