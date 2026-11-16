use serde_json::Value;

pub const MIN_SPEED_SAMPLES: i64 = 5;
const SPEED_BAND: f64 = 0.10;
const PRICE_EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preference {
    #[default]
    Balanced,
    Cheapest,
    Fastest,
    Latency,
}

impl Preference {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "balanced" => Some(Self::Balanced),
            "cheapest" | "price" => Some(Self::Cheapest),
            "fastest" | "throughput" => Some(Self::Fastest),
            "latency" => Some(Self::Latency),
            _ => None,
        }
    }

    pub fn from_setting(raw: &str) -> Option<Self> {
        Self::parse(raw).filter(|p| *p != Self::Latency)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Cheapest => "cheapest",
            Self::Fastest => "fastest",
            Self::Latency => "latency",
        }
    }

    pub fn from_provider_field(provider: Option<&Value>) -> Option<Self> {
        let sort = provider?.get("sort")?;
        let raw = sort.as_str().or_else(|| sort.get("by").and_then(Value::as_str))?;
        Self::parse(raw)
    }
}

pub fn split_model_suffix(model: &str) -> (&str, Option<Preference>) {
    for (suffix, preference) in [(":floor", Preference::Cheapest), (":nitro", Preference::Fastest)] {
        if let Some(bare) = model.strip_suffix(suffix) {
            if !bare.is_empty() {
                return (bare, Some(preference));
            }
        }
    }
    (model, None)
}

#[derive(Debug, Clone, Default)]
pub struct Stat {
    pub price: Option<f64>,
    pub tokens_per_second: Option<f64>,
    pub ttft_ms: Option<f64>,
    pub at_capacity: bool,
}

fn median(values: &mut Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 { values[mid] } else { (values[mid - 1] + values[mid]) / 2.0 })
}

fn filled(values: impl Iterator<Item = Option<f64>>) -> Vec<Option<f64>> {
    let values: Vec<Option<f64>> = values.collect();
    let mut known: Vec<f64> = values.iter().flatten().copied().collect();
    let fallback = median(&mut known);
    values.into_iter().map(|v| v.or(fallback)).collect()
}

fn move_to_front(order: &mut [usize], index: usize) {
    if let Some(at) = order.iter().position(|i| *i == index) {
        order[..=at].rotate_right(1);
    }
}

pub fn reorder(preference: Preference, stats: &[Stat], pinned: Option<usize>) -> Vec<usize> {
    let mut order: Vec<usize> = (0..stats.len()).collect();
    match preference {
        Preference::Balanced => {
            if let Some(pin) = pinned {
                move_to_front(&mut order, pin);
            }
        }
        Preference::Cheapest => {
            let price = |i: usize| stats[i].price.unwrap_or(f64::INFINITY);
            order.sort_by(|a, b| {
                let (pa, pb) = (price(*a), price(*b));
                if (pa - pb).abs() < PRICE_EPSILON || pa == pb { std::cmp::Ordering::Equal } else { pa.total_cmp(&pb) }
            });
            if let Some(pin) = pinned {
                let best = order.first().map(|i| price(*i)).unwrap_or(f64::INFINITY);
                if (price(pin) - best).abs() < PRICE_EPSILON || price(pin) == best {
                    move_to_front(&mut order, pin);
                }
            }
        }
        Preference::Fastest => {
            let tps = filled(stats.iter().map(|s| s.tokens_per_second));
            let ttft = filled(stats.iter().map(|s| s.ttft_ms));
            let key = |i: usize| (tps[i].unwrap_or(0.0), ttft[i].unwrap_or(f64::INFINITY));
            order.sort_by(|a, b| {
                let (ka, kb) = (key(*a), key(*b));
                kb.0.total_cmp(&ka.0).then(ka.1.total_cmp(&kb.1))
            });
            if let Some(pin) = pinned {
                let best = order.first().map(|i| key(*i).0).unwrap_or(0.0);
                if key(pin).0 >= best * (1.0 - SPEED_BAND) {
                    move_to_front(&mut order, pin);
                }
            }
        }
        Preference::Latency => {
            let tps = filled(stats.iter().map(|s| s.tokens_per_second));
            let ttft = filled(stats.iter().map(|s| s.ttft_ms));
            let key = |i: usize| (ttft[i].unwrap_or(f64::INFINITY), tps[i].unwrap_or(0.0));
            order.sort_by(|a, b| {
                let (ka, kb) = (key(*a), key(*b));
                ka.0.total_cmp(&kb.0).then(kb.1.total_cmp(&ka.1))
            });
            if let Some(pin) = pinned {
                let best = order.first().map(|i| key(*i).0).unwrap_or(f64::INFINITY);
                if key(pin).0 <= best * (1.0 + SPEED_BAND) {
                    move_to_front(&mut order, pin);
                }
            }
        }
    }
    let (open, full): (Vec<usize>, Vec<usize>) = order.into_iter().partition(|i| !stats[*i].at_capacity);
    open.into_iter().chain(full).collect()
}

pub fn resolve(
    request: Option<Preference>,
    per_model: Option<Preference>,
    user_default: Option<Preference>,
) -> Preference {
    request.or(per_model).or(user_default).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn price(p: f64) -> Stat {
        Stat { price: Some(p), ..Default::default() }
    }

    fn speed(tps: Option<f64>, ttft: Option<f64>) -> Stat {
        Stat { tokens_per_second: tps, ttft_ms: ttft, ..Default::default() }
    }

    #[test]
    fn suffixes_are_stripped_and_mapped() {
        assert_eq!(split_model_suffix("acme/m-1:floor"), ("acme/m-1", Some(Preference::Cheapest)));
        assert_eq!(split_model_suffix("acme/m-1:nitro"), ("acme/m-1", Some(Preference::Fastest)));
        assert_eq!(split_model_suffix("acme/m-1"), ("acme/m-1", None));
        assert_eq!(split_model_suffix(":floor"), (":floor", None));
    }

    #[test]
    fn provider_sort_accepts_a_string_or_an_object() {
        let string = serde_json::json!({"sort": "price"});
        let object = serde_json::json!({"sort": {"by": "throughput"}});
        let latency = serde_json::json!({"sort": "latency"});
        let junk = serde_json::json!({"sort": "vibes"});
        assert_eq!(Preference::from_provider_field(Some(&string)), Some(Preference::Cheapest));
        assert_eq!(Preference::from_provider_field(Some(&object)), Some(Preference::Fastest));
        assert_eq!(Preference::from_provider_field(Some(&latency)), Some(Preference::Latency));
        assert_eq!(Preference::from_provider_field(Some(&junk)), None);
        assert_eq!(Preference::from_provider_field(None), None);
    }

    #[test]
    fn precedence_is_request_then_model_then_user_then_balanced() {
        use Preference::*;
        assert_eq!(resolve(Some(Fastest), Some(Cheapest), Some(Balanced)), Fastest);
        assert_eq!(resolve(None, Some(Cheapest), Some(Fastest)), Cheapest);
        assert_eq!(resolve(None, None, Some(Fastest)), Fastest);
        assert_eq!(resolve(None, None, None), Balanced);
    }

    #[test]
    fn cheapest_picks_the_lowest_price_and_ties_keep_the_balanced_order() {
        let stats = [price(3.0), price(1.0), price(1.0), price(2.0)];
        assert_eq!(reorder(Preference::Cheapest, &stats, None), vec![1, 2, 3, 0]);
    }

    #[test]
    fn unpriced_members_come_after_priced_ones() {
        let stats = [Stat::default(), price(5.0)];
        assert_eq!(reorder(Preference::Cheapest, &stats, None), vec![1, 0]);
    }

    #[test]
    fn fastest_picks_the_highest_throughput_with_ttft_as_tie_break() {
        let stats = [speed(Some(50.0), Some(400.0)), speed(Some(80.0), Some(900.0)), speed(Some(80.0), Some(300.0))];
        assert_eq!(reorder(Preference::Fastest, &stats, None), vec![2, 1, 0]);
    }

    #[test]
    fn an_unsampled_member_is_treated_as_the_median() {
        let stats = [speed(Some(10.0), None), speed(None, None), speed(Some(100.0), None), speed(Some(40.0), None), speed(Some(20.0), None)];
        assert_eq!(reorder(Preference::Fastest, &stats, None), vec![2, 3, 1, 4, 0]);
    }

    #[test]
    fn latency_sorts_by_ttft() {
        let stats = [speed(Some(90.0), Some(800.0)), speed(Some(20.0), Some(100.0))];
        assert_eq!(reorder(Preference::Latency, &stats, None), vec![1, 0]);
    }

    #[test]
    fn the_pin_wins_only_within_the_same_price() {
        let stats = [price(1.0), price(1.0), price(2.0)];
        assert_eq!(reorder(Preference::Cheapest, &stats, Some(1)), vec![1, 0, 2]);
        assert_eq!(reorder(Preference::Cheapest, &stats, Some(2)), vec![0, 1, 2]);
    }

    #[test]
    fn the_pin_wins_within_ten_percent_of_the_best_speed() {
        let stats = [speed(Some(100.0), None), speed(Some(92.0), None), speed(Some(80.0), None)];
        assert_eq!(reorder(Preference::Fastest, &stats, Some(1)), vec![1, 0, 2]);
        assert_eq!(reorder(Preference::Fastest, &stats, Some(2)), vec![0, 1, 2]);
    }

    #[test]
    fn balanced_puts_the_pin_first() {
        let stats = [Stat::default(), Stat::default(), Stat::default()];
        assert_eq!(reorder(Preference::Balanced, &stats, Some(2)), vec![2, 0, 1]);
    }

    #[test]
    fn a_member_at_capacity_drops_behind_the_others() {
        let stats = [Stat { price: Some(1.0), at_capacity: true, ..Default::default() }, price(2.0), price(3.0)];
        assert_eq!(reorder(Preference::Cheapest, &stats, None), vec![1, 2, 0]);
    }
}
