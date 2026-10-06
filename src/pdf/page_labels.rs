use super::PdfError;
use lopdf::{Document, Object};

pub const MAX_PAGE_LABEL_CHARS: usize = 256;

fn error(message: impl std::fmt::Display) -> PdfError {
    PdfError::Edit(format!("invalid PDF page labels: {message}"))
}

fn resolve<'a>(doc: &'a Document, mut object: &'a Object) -> Result<&'a Object, PdfError> {
    let mut seen = std::collections::HashSet::new();
    while let Object::Reference(id) = object {
        if !seen.insert(*id) || seen.len() > 128 {
            return Err(error("cyclic or oversized reference chain"));
        }
        object = doc.get_object(*id).map_err(error)?;
    }
    Ok(object)
}

fn suffix(style: &[u8], mut n: u64) -> Result<String, PdfError> {
    match style {
        b"D" => Ok(n.to_string()),
        b"A" | b"a" => {
            let repeat = (n - 1) / 26 + 1;
            if repeat > 16384 {
                return Err(error("alphabetic label exceeds limit"));
            }
            let base = if style == b"A" { b'A' } else { b'a' };
            Ok(char::from(base + ((n - 1) % 26) as u8)
                .to_string()
                .repeat(repeat as usize))
        }
        b"R" | b"r" => {
            if n > 16_000_000 {
                return Err(error("Roman label exceeds limit"));
            }
            let mut result = String::new();
            for (value, text) in [
                (1000, "M"),
                (900, "CM"),
                (500, "D"),
                (400, "CD"),
                (100, "C"),
                (90, "XC"),
                (50, "L"),
                (40, "XL"),
                (10, "X"),
                (9, "IX"),
                (5, "V"),
                (4, "IV"),
                (1, "I"),
            ] {
                while n >= value {
                    result.push_str(text);
                    n -= value;
                }
            }
            if style == b"r" {
                result.make_ascii_lowercase();
            }
            Ok(result)
        }
        _ => Err(error("unknown numbering style")),
    }
}

pub(super) fn read(doc: &Document) -> Result<Vec<String>, PdfError> {
    let count = doc.get_pages().len();
    let catalog = doc.catalog().map_err(error)?;
    let Ok(root) = catalog.get(b"PageLabels") else {
        return Ok((1..=count).map(|i| i.to_string()).collect());
    };
    // Exit frames validate each subtree's Limits after all its leaves.
    let mut stack = vec![(root, 0usize, None::<usize>)];
    let mut seen = std::collections::HashSet::new();
    let mut ranges: Vec<(usize, &lopdf::Dictionary)> = Vec::new();
    let mut nodes = 0;
    while let Some((node, depth, exit_start)) = stack.pop() {
        if let Some(start) = exit_start {
            let dict = resolve(doc, node)?.as_dict().map_err(error)?;
            if let Ok(limits) = dict.get(b"Limits") {
                let limits = resolve(doc, limits)?.as_array().map_err(error)?;
                if limits.len() != 2 {
                    return Err(error("invalid Limits array"));
                }
                let lower = resolve(doc, &limits[0])?.as_i64().map_err(error)?;
                let upper = resolve(doc, &limits[1])?.as_i64().map_err(error)?;
                if ranges.get(start).map(|r| r.0 as i64) != Some(lower)
                    || ranges.last().map(|r| r.0 as i64) != Some(upper)
                {
                    return Err(error("Limits do not match subtree keys"));
                }
            }
            continue;
        }
        nodes += 1;
        if depth > 128 || nodes > 10000 {
            return Err(error("number tree exceeds traversal limit"));
        }
        if let Object::Reference(id) = node
            && !seen.insert(*id)
        {
            return Err(error("cyclic or shared number tree node"));
        }
        let dict = resolve(doc, node)?.as_dict().map_err(error)?;
        match (dict.get(b"Nums").ok(), dict.get(b"Kids").ok()) {
            // Validation is deferred until descendants have supplied keys.
            (Some(nums), None) => {
                stack.push((node, depth, Some(ranges.len())));
                let nums = resolve(doc, nums)?.as_array().map_err(error)?;
                if nums.is_empty() || nums.len() % 2 != 0 || nums.len() / 2 > count {
                    return Err(error("invalid Nums array"));
                }
                let mut previous = None;
                for pair in nums.as_chunks::<2>().0 {
                    let key = resolve(doc, &pair[0])?.as_i64().map_err(error)?;
                    let key = usize::try_from(key).map_err(error)?;
                    if key >= count || previous.is_some_and(|p| key <= p) {
                        return Err(error("invalid number tree index"));
                    }
                    previous = Some(key);
                    let label = resolve(doc, &pair[1])?.as_dict().map_err(error)?;
                    if ranges.last().is_some_and(|r| key <= r.0) {
                        return Err(error("duplicate or unordered number tree index"));
                    }
                    ranges.push((key, label));
                }
            }
            (None, Some(kids)) => {
                let kids = resolve(doc, kids)?.as_array().map_err(error)?;
                if kids.is_empty() || kids.len() > 10000 {
                    return Err(error("invalid Kids array"));
                }
                stack.push((node, depth, Some(ranges.len())));
                stack.extend(kids.iter().rev().map(|o| (o, depth + 1, None)));
            }
            _ => return Err(error("number tree must contain Nums or Kids")),
        }
    }
    if ranges.first().map(|r| r.0) != Some(0) {
        return Err(error("first range must begin at page zero"));
    }
    let mut result = Vec::with_capacity(count);
    let mut total = 0usize;
    let mut iter = ranges.iter().peekable();
    while let Some(&(start, dict)) = iter.next() {
        let end = iter.peek().map(|(key, _)| *key).unwrap_or(count);
        let prefix = match dict.get(b"P") {
            Ok(p) => {
                let p = resolve(doc, p)?;
                let bytes = p.as_str().map_err(error)?;
                if bytes.len() > 65536 {
                    return Err(error("encoded prefix exceeds limit"));
                }
                if bytes.starts_with(&[0xfe, 0xff]) && bytes.len() % 2 != 0 {
                    return Err(error("truncated UTF-16 prefix"));
                }
                if let Some(text) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
                    std::str::from_utf8(text).map_err(error)?.to_owned()
                } else {
                    lopdf::decode_text_string(p).map_err(error)?
                }
            }
            Err(_) => String::new(),
        };
        if prefix.len() > 65536 {
            return Err(error("prefix exceeds limit"));
        }
        let style = dict
            .get(b"S")
            .ok()
            .map(|s| resolve(doc, s).and_then(|s| s.as_name().map_err(error)))
            .transpose()?;
        let initial = match dict.get(b"St") {
            Ok(st) => resolve(doc, st)?.as_i64().map_err(error)?,
            Err(_) => 1,
        };
        if initial < 1 {
            return Err(error("St must be positive"));
        }
        for index in start..end {
            let mut label = prefix.clone();
            if let Some(style) = style {
                let n = (initial as u64)
                    .checked_add((index - start) as u64)
                    .ok_or_else(|| error("label number overflow"))?;
                label.push_str(&suffix(style, n)?);
            }
            total = total
                .checked_add(label.len())
                .ok_or_else(|| error("label size overflow"))?;
            if label.len() > 65536 || total > 16 * 1024 * 1024 {
                return Err(error("label text exceeds limit"));
            }
            result.push(label);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;
    fn document(count: usize) -> Document {
        let mut d = Document::with_version("1.7");
        let pages = d.new_object_id();
        let kids: Vec<Object> = (0..count)
            .map(|_| {
                d.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages})
                    .into()
            })
            .collect();
        d.objects.insert(
            pages,
            dictionary! {"Type"=>"Pages", "Kids"=>kids, "Count"=>count as i64}.into(),
        );
        let root = d.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
        d.trailer.set("Root", root);
        d
    }
    #[test]
    fn rejects_malformed_ranges_and_bounded_expansion() {
        for range in [
            dictionary! {"S"=>"Unknown"},
            dictionary! {"S"=>"D", "St"=>0},
            dictionary! {"S"=>"D", "St"=>-1},
            dictionary! {"P"=>Object::Null},
            dictionary! {"P"=>Object::String(vec![0xfe,0xff,0xd8,0x00], lopdf::StringFormat::Hexadecimal)},
            dictionary! {"S"=>"A", "St"=>i64::MAX},
            dictionary! {"S"=>"R", "St"=>i64::MAX},
        ] {
            let mut d = document(1);
            d.catalog_mut().unwrap().set(
                "PageLabels",
                dictionary! {"Nums"=>vec![0.into(), range.into()]},
            );
            assert!(read(&d).is_err());
        }
        for nums in [
            vec![0.into()],
            vec![(-1).into(), dictionary! {"S"=>"D"}.into()],
            vec![1.into(), dictionary! {"S"=>"D"}.into()],
            vec![
                0.into(),
                dictionary! {"S"=>"D"}.into(),
                0.into(),
                dictionary! {"S"=>"D"}.into(),
            ],
        ] {
            let mut d = document(2);
            d.catalog_mut()
                .unwrap()
                .set("PageLabels", dictionary! {"Nums"=>nums});
            assert!(read(&d).is_err());
        }
        let mut d = document(1);
        let mut root =
            d.add_object(dictionary! {"Nums"=>vec![0.into(), dictionary!{"S"=>"D"}.into()]});
        for _ in 0..130 {
            root = d.add_object(dictionary! {"Kids"=>vec![Object::Reference(root)]});
        }
        d.catalog_mut().unwrap().set("PageLabels", root);
        assert!(read(&d).is_err());
    }
    #[test]
    fn empty_prefix_and_indirect_range_fields_are_valid() {
        let mut d = document(2);
        let style = d.add_object(Object::Name(b"a".to_vec()));
        let start = d.add_object(Object::Integer(53));
        let range = d.add_object(dictionary! {"S"=>style, "St"=>start});
        let empty = d.add_object(dictionary! {"P"=>Object::string_literal("")});
        d.catalog_mut().unwrap().set("PageLabels", dictionary!{"Nums"=>vec![0.into(), Object::Reference(range), 1.into(), Object::Reference(empty)]});
        assert_eq!(read(&d).unwrap(), ["aaa", ""]);
    }
    #[test]
    fn rejects_inconsistent_number_tree_limits() {
        let mut d = document(2);
        d.catalog_mut().unwrap().set("PageLabels", dictionary!{"Nums"=>vec![0.into(), dictionary!{"S"=>"D"}.into()], "Limits"=>vec![0.into(),1.into()]});
        assert!(
            read(&d).is_err(),
            "Limits must describe range keys, not page extent"
        );
    }
    #[test]
    fn reads_utf8_bom_prefix_without_exposing_bom() {
        let mut d = document(2);
        let prefix = Object::String(b"\xef\xbb\xbfPlan".to_vec(), lopdf::StringFormat::Literal);
        d.catalog_mut().unwrap().set(
            "PageLabels",
            dictionary! {"Nums"=>vec![0.into(), dictionary!{"P"=>prefix}.into()]},
        );
        assert_eq!(read(&d).unwrap(), ["Plan", "Plan"]);
    }
    #[test]
    fn rejects_truncated_utf16_prefix() {
        let mut d = document(1);
        let prefix = Object::String(vec![0xfe, 0xff, 0x41], lopdf::StringFormat::Hexadecimal);
        d.catalog_mut().unwrap().set(
            "PageLabels",
            dictionary! {"Nums"=>vec![0.into(), dictionary!{"P"=>prefix}.into()]},
        );
        assert!(
            read(&d).is_err(),
            "odd UTF-16 is malformed, not a padded label"
        );
    }
    #[test]
    fn reads_indirect_number_tree_and_all_styles() {
        let mut d = document(10);
        let prefix = d.add_object(Object::string_literal("Sec-"));
        let nums = d.add_object(vec![
            0.into(),
            dictionary! {"S"=>"D", "P"=>prefix, "St"=>3}.into(),
            2.into(),
            dictionary! {"S"=>"r", "St"=>4}.into(),
            4.into(),
            dictionary! {"S"=>"R", "St"=>9}.into(),
            6.into(),
            dictionary! {"S"=>"A", "St"=>26}.into(),
            8.into(),
            dictionary! {"S"=>"a", "St"=>27}.into(),
        ]);
        let leaf = d.add_object(dictionary! {"Nums"=>nums, "Limits"=>vec![0.into(),8.into()]});
        let kids = d.add_object(vec![Object::Reference(leaf)]);
        let root = d.add_object(dictionary! {"Kids"=>kids});
        d.catalog_mut().unwrap().set("PageLabels", root);
        assert_eq!(
            read(&d).unwrap(),
            [
                "Sec-3", "Sec-4", "iv", "v", "IX", "X", "Z", "AA", "aa", "bb"
            ]
        );
    }
}
