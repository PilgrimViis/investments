use std::str::FromStr;

#[cfg(test)] use mockito;
use reqwest::{self, Url};
use serde_xml_rs;

use core::GenericResult;
use currency::CurrencyRate;
use types::{Date, Decimal};
use util;

#[cfg(not(test))]
const CBR_URL: &'static str = "http://www.cbr.ru";

#[cfg(test)]
const CBR_URL: &'static str = mockito::SERVER_URL;

pub fn get_rates(currency: &str, start_date: Date, end_date: Date) -> GenericResult<Vec<CurrencyRate>> {
    let currency_code = "R01235"; // HACK: Don't hardcode
    if currency != "USD" {
        return Err!("{} currency is not supported yet.", currency);
    }

    let date_format = "%d/%m/%Y";
    let start_date_string = start_date.format(date_format).to_string();
    let end_date_string = end_date.format(date_format).to_string();

    let url = Url::parse_with_params(
        &(CBR_URL.to_owned() + "/scripts/XML_dynamic.asp"),
        &[
            ("date_req1", start_date_string.as_ref()),
            ("date_req2", end_date_string.as_ref()),
            ("VAL_NM_RQ", currency_code),
        ],
    )?;

    let get = |url| -> GenericResult<Vec<CurrencyRate>> {
        debug!("Getting {} currency rates for {} - {}...", currency, start_date, end_date);

        let mut response = reqwest::Client::new().get(url).send()?;
        if !response.status().is_success() {
            return Err!("The server returned an error: {}", response.status());
        }

        Ok(parse_rates(start_date, end_date, &response.text()?).map_err(|e| format!(
            "Rates info parsing error: {}", e))?)
    };

    Ok(get(url.as_str()).map_err(|e| format!(
        "Failed to get currency rates from {}: {}", url, e))?)
}

fn parse_rates(start_date: Date, end_date: Date, data: &str) -> GenericResult<Vec<CurrencyRate>> {
    #[derive(Deserialize)]
    struct Rate {
        #[serde(rename = "Date")]
        date: String,

        #[serde(rename = "Nominal")]
        lot: i32,

        #[serde(rename = "Value")]
        price: String,
    }

    #[derive(Deserialize)]
    struct Rates {
        #[serde(rename = "DateRange1")]
        start_date: String,

        #[serde(rename = "DateRange2")]
        end_date: String,

        #[serde(rename = "Record", default)]
        rates: Vec<Rate>
    }

    let date_format = "%d.%m.%Y";
    let result: Rates = serde_xml_rs::deserialize(data.as_bytes())?;

    if util::parse_date(&result.start_date, date_format)? != start_date ||
        util::parse_date(&result.end_date, date_format)? != end_date {
        return Err!("The server returned currency rates info for an invalid period");
    }

    let mut rates = Vec::with_capacity(result.rates.len());

    for rate in result.rates {
        let lot = rate.lot;
        if lot <= 0 {
            return Err!("Invalid lot: {}", lot);
        }

        let price = rate.price.replace(",", ".");
        let price = Decimal::from_str(&price).map_err(|_| format!(
            "Invalid price: {:?}", rate.price))?;

        rates.push(CurrencyRate {
            date: util::parse_date(&rate.date, date_format)?,
            price: price / lot,
        })
    }

    Ok(rates)
}

#[cfg(test)]
mod tests {
    use mockito::{Mock, mock};

    use super::*;

    #[test]
    fn empty_rates() {
        let _mock = mock_cbr_response(
            "/scripts/XML_dynamic.asp?date_req1=02%2F09%2F2018&date_req2=03%2F09%2F2018&VAL_NM_RQ=R01235",
            indoc!(r#"
                <?xml version="1.0" encoding="windows-1251"?>
                <ValCurs ID="R01235" DateRange1="02.09.2018" DateRange2="03.09.2018" name="Foreign Currency Market Dynamic">
                </ValCurs>
            "#)
        );

        assert_eq!(
            get_rates("USD", Date::from_ymd(2018, 9, 2), Date::from_ymd(2018, 9, 3)).unwrap(),
            vec![],
        );
    }

    #[test]
    fn rates() {
        let _mock = mock_cbr_response(
            "/scripts/XML_dynamic.asp?date_req1=01%2F09%2F2018&date_req2=04%2F09%2F2018&VAL_NM_RQ=R01235",
            indoc!(r#"
                <?xml version="1.0" encoding="windows-1251"?>
                <ValCurs ID="R01235" DateRange1="01.09.2018" DateRange2="04.09.2018" name="Foreign Currency Market Dynamic">
                    <Record Date="01.09.2018" Id="R01235">
                        <Nominal>1</Nominal>
                        <Value>68,0447</Value>
                    </Record>
                    <Record Date="04.09.2018" Id="R01235">
                        <Nominal>1</Nominal>
                        <Value>67,7443</Value>
                    </Record>
                </ValCurs>
            "#)
        );

        assert_eq!(
            get_rates("USD", Date::from_ymd(2018, 9, 1), Date::from_ymd(2018, 9, 4)).unwrap(),
            vec![CurrencyRate {
                date: Date::from_ymd(2018, 9, 1),
                price: Decimal::from_str("68.0447").unwrap(),
            }, CurrencyRate {
                date: Date::from_ymd(2018, 9, 4),
                price: Decimal::from_str("67.7443").unwrap(),
            }],
        );
    }

    fn mock_cbr_response(path: &str, data: &str) -> Mock {
        return mock("GET", path)
            .with_status(200)
            .with_header("Content-Type", "application/xml; charset=windows-1251")
            .with_body(data)
            .create();
    }
}