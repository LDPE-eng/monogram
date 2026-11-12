use std::{collections::BTreeMap, error::Error, fmt};

pub const SUPPORTED_LOCALES: [&str; 10] = [
    "en", "ru", "es", "hy", "ja", "pt-BR", "sk", "tr", "uk", "zh",
];

const RESOURCES: [(&str, &str); 10] = [
    (
        include_str!("../locales/en/monogram.ftl"),
        include_str!("../locales/en/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/ru/monogram.ftl"),
        include_str!("../locales/ru/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/es/monogram.ftl"),
        include_str!("../locales/es/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/hy/monogram.ftl"),
        include_str!("../locales/hy/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/ja/monogram.ftl"),
        include_str!("../locales/ja/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/pt-BR/monogram.ftl"),
        include_str!("../locales/pt-BR/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/sk/monogram.ftl"),
        include_str!("../locales/sk/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/tr/monogram.ftl"),
        include_str!("../locales/tr/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/uk/monogram.ftl"),
        include_str!("../locales/uk/desktop-extra.ftl"),
    ),
    (
        include_str!("../locales/zh/monogram.ftl"),
        include_str!("../locales/zh/desktop-extra.ftl"),
    ),
];

#[derive(Debug, PartialEq, Eq)]
pub enum I18nError {
    UnknownKey(String),
    WrongKind(String),
    MissingArgument(String),
    InvalidResource,
}

impl fmt::Display for I18nError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(key) => write!(formatter, "Unknown localization key: {key}"),
            Self::WrongKind(key) => {
                write!(formatter, "Incorrect localization resource kind: {key}")
            }
            Self::MissingArgument(name) => {
                write!(formatter, "Missing localization argument: {name}")
            }
            Self::InvalidResource => formatter.write_str("Invalid generated localization resource"),
        }
    }
}

impl Error for I18nError {}

enum Message {
    Text(&'static str),
    Plural(Vec<(&'static str, &'static str)>),
}

pub struct I18n {
    locale: &'static str,
    messages: BTreeMap<&'static str, Message>,
}

impl I18n {
    pub fn new(locale: &str) -> Result<Self, I18nError> {
        let normalized = locale
            .split(['.', '@'])
            .next()
            .unwrap_or_default()
            .replace('_', "-")
            .to_ascii_lowercase();
        let language = normalized.split('-').next().unwrap_or_default();
        let selected = if language == "pt" && (normalized == "pt" || normalized == "pt-br") {
            "pt-BR"
        } else if language != "pt" && SUPPORTED_LOCALES.contains(&language) {
            language
        } else {
            "en"
        };
        let index = SUPPORTED_LOCALES
            .iter()
            .position(|candidate| *candidate == selected)
            .ok_or(I18nError::InvalidResource)?;
        let mut messages = parse_resource(RESOURCES[index].0)?;
        messages.extend(parse_resource(RESOURCES[index].1)?);
        Ok(Self {
            locale: SUPPORTED_LOCALES[index],
            messages,
        })
    }

    pub fn locale(&self) -> &'static str {
        self.locale
    }

    pub fn text(&self, key: &str, args: &[(&str, &str)]) -> Result<String, I18nError> {
        match self.messages.get(key) {
            Some(Message::Text(pattern)) => render(pattern, args),
            Some(Message::Plural(_)) => Err(I18nError::WrongKind(key.to_owned())),
            None => Err(I18nError::UnknownKey(key.to_owned())),
        }
    }

    // Android quantity selection is independent of the printf arguments.
    pub fn plural(
        &self,
        key: &str,
        count: u64,
        args: &[(&str, &str)],
    ) -> Result<String, I18nError> {
        match self.messages.get(key) {
            Some(Message::Plural(variants)) => {
                let category = integer_quantity(self.locale, count);
                let pattern = variants
                    .iter()
                    .find(|(quantity, _)| *quantity == category)
                    .or_else(|| variants.iter().find(|(quantity, _)| *quantity == "other"))
                    .ok_or(I18nError::InvalidResource)?
                    .1;
                render(pattern, args)
            }
            Some(Message::Text(_)) => Err(I18nError::WrongKind(key.to_owned())),
            None => Err(I18nError::UnknownKey(key.to_owned())),
        }
    }
}

fn integer_quantity(locale: &str, count: u64) -> &'static str {
    match locale {
        "ru" | "uk" => match (count % 10, count % 100) {
            (1, n) if n != 11 => "one",
            (2..=4, n) if !(12..=14).contains(&n) => "few",
            _ => "many",
        },
        "sk" => match count {
            1 => "one",
            2..=4 => "few",
            _ => "other",
        },
        "hy" | "pt-BR" if count <= 1 => "one",
        "es" | "pt-BR" if count != 0 && count.is_multiple_of(1_000_000) => "many",
        "en" | "es" | "tr" if count == 1 => "one",
        _ => "other",
    }
}

// This reads only the converter's literal/variable/select subset, not arbitrary Fluent.
fn parse_resource(resource: &'static str) -> Result<BTreeMap<&'static str, Message>, I18nError> {
    let mut messages = BTreeMap::new();
    let mut lines = resource.lines();
    while let Some(line) = lines.next() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(" = ").ok_or(I18nError::InvalidResource)?;
        let message = if value == "{ $count ->" {
            let mut variants = Vec::new();
            let mut closed = false;
            for line in lines.by_ref() {
                if line == "    }" {
                    closed = true;
                    break;
                }
                let line = line
                    .strip_prefix("    ")
                    .ok_or(I18nError::InvalidResource)?;
                let (default, line) = match line.strip_prefix('*') {
                    Some(line) => (true, line),
                    None => (false, line),
                };
                let (quantity, pattern) = line
                    .strip_prefix('[')
                    .and_then(|line| line.split_once("] "))
                    .ok_or(I18nError::InvalidResource)?;
                if !["zero", "one", "two", "few", "many", "other"].contains(&quantity)
                    || default != (quantity == "other")
                    || variants.iter().any(|(existing, _)| *existing == quantity)
                {
                    return Err(I18nError::InvalidResource);
                }
                variants.push((quantity, pattern));
            }
            if !closed || !variants.iter().any(|(quantity, _)| *quantity == "other") {
                return Err(I18nError::InvalidResource);
            }
            Message::Plural(variants)
        } else {
            Message::Text(value)
        };
        if messages.insert(key, message).is_some() {
            return Err(I18nError::InvalidResource);
        }
    }
    Ok(messages)
}

fn render(mut pattern: &str, args: &[(&str, &str)]) -> Result<String, I18nError> {
    let mut output = String::new();
    while !pattern.is_empty() {
        if let Some(literal) = pattern.strip_prefix("{ \"") {
            let mut chars = literal.char_indices();
            let mut closed = None;
            while let Some((position, char)) = chars.next() {
                match char {
                    '"' => {
                        closed = Some(position + 1);
                        break;
                    }
                    '\\' => match chars.next().map(|(_, char)| char) {
                        Some(char @ ('"' | '\\')) => output.push(char),
                        Some('u') => {
                            let mut digits = String::new();
                            for _ in 0..4 {
                                digits.push(chars.next().ok_or(I18nError::InvalidResource)?.1);
                            }
                            let code = u32::from_str_radix(&digits, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or(I18nError::InvalidResource)?;
                            output.push(code);
                        }
                        _ => return Err(I18nError::InvalidResource),
                    },
                    char => output.push(char),
                }
            }
            pattern = literal
                .get(closed.ok_or(I18nError::InvalidResource)?..)
                .and_then(|tail| tail.strip_prefix(" }"))
                .ok_or(I18nError::InvalidResource)?;
        } else if let Some(variable) = pattern.strip_prefix("{ $") {
            let (name, tail) = variable
                .split_once(" }")
                .ok_or(I18nError::InvalidResource)?;
            let value = args
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .ok_or_else(|| I18nError::MissingArgument(name.to_owned()))?
                .1;
            output.push_str(value);
            pattern = tail;
        } else {
            return Err(I18nError::InvalidResource);
        }
    }
    Ok(output)
}
