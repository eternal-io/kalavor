use chrono::{DateTime, Datelike, Local};
use radix_fmt::radix_36;

/// `_human` means "unfriendly to file names and URLs".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Katetime {
    pub show_date: bool,
    pub show_time: bool,
    pub url_safe: bool,
}

impl Katetime {
    /// 不要公开，强制人们使用结构体语法来构建，以期得到更易懂的代码。
    fn new(show_date: bool, show_time: bool, url_safe: bool) -> Self {
        Self {
            show_date,
            show_time,
            url_safe,
        }
    }

    pub fn now_date_human() -> String {
        Self::new(true, false, false).generate_now()
    }
    pub fn now_time_human() -> String {
        Self::new(false, true, false).generate_now()
    }
    pub fn now_datetime_human() -> String {
        Self::new(true, true, false).generate_now()
    }

    pub fn now_date() -> String {
        Self::new(true, false, true).generate_now()
    }
    pub fn now_time() -> String {
        Self::new(false, true, true).generate_now()
    }
    pub fn now_datetime() -> String {
        Self::new(true, true, true).generate_now()
    }

    pub fn generate_now(&self) -> String {
        self.generate(&Local::now())
    }
    pub fn generate(&self, dt: &DateTime<Local>) -> String {
        if !self.show_date && !self.show_time {
            return String::with_capacity(0);
        }

        let mut s = String::with_capacity(17);

        if self.show_date {
            s.push_str(&match dt.year() - 2022 {
                yr @ i32::MIN..=-1 => {
                    format!(
                        "{}{}",
                        radix_36(yr.rem_euclid(200) / 10 + 10),
                        yr.rem_euclid(10),
                    )
                }
                yr => format!("{:#}{}", radix_36((yr % 200) / 10 + 10), yr % 10),
            });
            s.push_str(&match self.url_safe {
                true => dt.format("%m%d").to_string(),
                false => dt.format("~%m/%d").to_string(),
            });
            if self.show_time {
                s.push_str(match self.url_safe {
                    true => "-",
                    false => " ",
                });
            }
        }

        if self.show_time {
            match self.url_safe {
                true => {
                    s.push_str(&dt.format("%H%M-%S").to_string());
                    s.push_str(&format!("{:02}", dt.timestamp_subsec_millis() / 10));
                }
                false => s.push_str(&dt.format("%H:%M:%S").to_string()),
            }
        }

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

    #[test]
    fn test() {
        let kt_safe = Katetime::new(true, true, true);
        let kt_human = Katetime::new(true, true, false);

        let time = NaiveTime::from_hms_milli_opt(4, 56, 7, 890).unwrap();

        let date = [
            NaiveDate::from_ymd_opt(1970, 11, 23),
            NaiveDate::from_ymd_opt(2000, 11, 23),
            NaiveDate::from_ymd_opt(2010, 11, 23),
            NaiveDate::from_ymd_opt(2021, 11, 23),
            NaiveDate::from_ymd_opt(2022, 11, 23),
            NaiveDate::from_ymd_opt(2033, 11, 23),
            NaiveDate::from_ymd_opt(2221, 11, 23),
            NaiveDate::from_ymd_opt(2222, 11, 23),
        ];

        date.iter()
            .map(|date| {
                NaiveDateTime::new(date.unwrap(), time)
                    .and_local_timezone(Local)
                    .unwrap()
            })
            .zip(
                [
                    "o81123-0456-0789",
                    "r81123-0456-0789",
                    "s81123-0456-0789",
                    "t91123-0456-0789",
                    "A01123-0456-0789",
                    "B11123-0456-0789",
                    "T91123-0456-0789",
                    "A01123-0456-0789",
                ]
                .into_iter(),
            )
            .for_each(|(dt, ans)| assert_eq!(kt_safe.generate(&dt), ans));

        date.iter()
            .map(|date| {
                NaiveDateTime::new(date.unwrap(), time)
                    .and_local_timezone(Local)
                    .unwrap()
            })
            .zip(
                [
                    "o8~11/23 04:56:07",
                    "r8~11/23 04:56:07",
                    "s8~11/23 04:56:07",
                    "t9~11/23 04:56:07",
                    "A0~11/23 04:56:07",
                    "B1~11/23 04:56:07",
                    "T9~11/23 04:56:07",
                    "A0~11/23 04:56:07",
                ]
                .into_iter(),
            )
            .for_each(|(dt, ans)| assert_eq!(kt_human.generate(&dt), ans));
    }
}
