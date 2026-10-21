# iOS 26 Typography Specification (Text Styles)

> Источник: Figma Canvas `Text styles` (Node ID: `0:2194`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Гарнитура шрифта: **SF Pro** (Apple System Font).  
> Все параметры извлечены без округлений и модификаций. Сырые данные сохранены в `docs/figma/raw/text_styles_0_2194.txt`.

---

## 1. Сводная таблица текстовых стилей

| Стиль | Вариант | Размер (`fontSize`) | Межстрочный интервал (`lineHeight`) | Начертание (`fontStyle` / `fontWeight`) | Трекинг (`letterSpacing`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Large Title** | Regular | `34px` | `41px` | Regular (`400`) | `0.0118em` (`+0.40px`) |
| **Large Title** | Emphasized | `34px` | `41px` | Bold (`700`) | `0.0118em` (`+0.40px`) |
| **Title 1** | Regular | `28px` | `34px` | Regular (`400`) | `0.0136em` (`+0.38px`) |
| **Title 1** | Emphasized | `28px` | `34px` | Bold (`700`) | `0.0136em` (`+0.38px`) |
| **Title 2** | Regular | `22px` | `28px` | Regular (`400`) | `-0.0118em` (`-0.26px`) |
| **Title 2** | Emphasized | `22px` | `28px` | Bold (`700`) | `-0.0118em` (`-0.26px`) |
| **Title 3** | Regular | `20px` | `25px` | Regular (`400`) | `-0.0225em` (`-0.45px`) |
| **Title 3** | Emphasized | `20px` | `25px` | Semibold (`590`) | `-0.0225em` (`-0.45px`) |
| **Headline** | Regular | `17px` | `22px` | Semibold (`590`) | `-0.0253em` (`-0.43px`) |
| **Headline** | Italic | `17px` | `22px` | Semibold Italic (`590`, italic) | `-0.0253em` (`-0.43px`) |
| **Body** | Regular | `17px` | `22px` | Regular (`400`) | `-0.0253em` (`-0.43px`) |
| **Body** | Emphasized | `17px` | `22px` | Semibold (`590`) | `-0.0253em` (`-0.43px`) |
| **Body** | Italic | `17px` | `22px` | Regular Italic (`400`, italic) | `-0.0253em` (`-0.43px`) |
| **Body** | Emphasized Italic | `17px` | `22px` | Semibold Italic (`590`, italic) | `-0.0253em` (`-0.43px`) |
| **Callout** | Regular | `16px` | `21px` | Regular (`400`) | `-0.0194em` (`-0.31px`) |
| **Callout** | Emphasized | `16px` | `21px` | Semibold (`590`) | `-0.0194em` (`-0.31px`) |
| **Callout** | Italic | `16px` | `21px` | Regular Italic (`400`, italic) | `-0.0194em` (`-0.31px`) |
| **Callout** | Emphasized Italic | `16px` | `21px` | Semibold Italic (`590`, italic) | `-0.0194em` (`-0.31px`) |
| **Subheadline** | Regular | `15px` | `20px` | Regular (`400`) | `-0.0153em` (`-0.23px`) |
| **Subheadline** | Emphasized | `15px` | `20px` | Semibold (`590`) | `-0.0153em` (`-0.23px`) |
| **Subheadline** | Italic | `15px` | `20px` | Regular Italic (`400`, italic) | `-0.0153em` (`-0.23px`) |
| **Subheadline** | Emphasized Italic | `15px` | `20px` | Semibold Italic (`590`, italic) | `-0.0153em` (`-0.23px`) |
| **Footnote** | Regular | `13px` | `18px` | Regular (`400`) | `-0.0062em` (`-0.08px`) |
| **Footnote** | Emphasized | `13px` | `18px` | Semibold (`590`) | `-0.0062em` (`-0.08px`) |
| **Footnote** | Italic | `13px` | `18px` | Regular Italic (`400`, italic) | `-0.0062em` (`-0.08px`) |
| **Footnote** | Emphasized Italic | `13px` | `18px` | Semibold Italic (`590`, italic) | `-0.0062em` (`-0.08px`) |
| **Caption 1** | Regular | `12px` | `16px` | Regular (`400`) | `0em` (`0px`) |
| **Caption 1** | Emphasized | `12px` | `16px` | Semibold (`590`) | `0em` (`0px`) |
| **Caption 1** | Italic | `12px` | `16px` | Regular Italic (`400`, italic) | `0em` (`0px`) |
| **Caption 1** | Emphasized Italic | `12px` | `16px` | Semibold Italic (`590`, italic) | `0em` (`0px`) |
| **Caption 2** | Regular | `11px` | `13px` | Regular (`400`) | `0.0055em` (`+0.06px`) |
| **Caption 2** | Emphasized | `11px` | `13px` | Semibold (`590`) | `0.0055em` (`+0.06px`) |
| **Caption 2** | Italic | `11px` | `13px` | Regular Italic (`400`, italic) | `0.0055em` (`+0.06px`) |
| **Caption 2** | Emphasized Italic | `11px` | `13px` | Semibold Italic (`590`, italic) | `0.0055em` (`+0.06px`) |

---

## 2. CSS Snippets

```css
/* Typography tokens */
--font-large-title-regular: 400 34px/41px 'SF Pro', -apple-system, sans-serif;
--tracking-large-title-regular: 0.0118em;

--font-large-title-emphasized: 700 34px/41px 'SF Pro', -apple-system, sans-serif;
--tracking-large-title-emphasized: 0.0118em;

--font-title-1-regular: 400 28px/34px 'SF Pro', -apple-system, sans-serif;
--tracking-title-1-regular: 0.0136em;

--font-title-1-emphasized: 700 28px/34px 'SF Pro', -apple-system, sans-serif;
--tracking-title-1-emphasized: 0.0136em;

--font-title-2-regular: 400 22px/28px 'SF Pro', -apple-system, sans-serif;
--tracking-title-2-regular: -0.0118em;

--font-title-2-emphasized: 700 22px/28px 'SF Pro', -apple-system, sans-serif;
--tracking-title-2-emphasized: -0.0118em;

--font-title-3-regular: 400 20px/25px 'SF Pro', -apple-system, sans-serif;
--tracking-title-3-regular: -0.0225em;

--font-title-3-emphasized: 590 20px/25px 'SF Pro', -apple-system, sans-serif;
--tracking-title-3-emphasized: -0.0225em;

--font-headline: 590 17px/22px 'SF Pro', -apple-system, sans-serif;
--tracking-headline: -0.0253em;

--font-body-regular: 400 17px/22px 'SF Pro', -apple-system, sans-serif;
--tracking-body-regular: -0.0253em;

--font-body-emphasized: 590 17px/22px 'SF Pro', -apple-system, sans-serif;
--tracking-body-emphasized: -0.0253em;

--font-callout-regular: 400 16px/21px 'SF Pro', -apple-system, sans-serif;
--tracking-callout-regular: -0.0194em;

--font-callout-emphasized: 590 16px/21px 'SF Pro', -apple-system, sans-serif;
--tracking-callout-emphasized: -0.0194em;

--font-subheadline-regular: 400 15px/20px 'SF Pro', -apple-system, sans-serif;
--tracking-subheadline-regular: -0.0153em;

--font-subheadline-emphasized: 590 15px/20px 'SF Pro', -apple-system, sans-serif;
--tracking-subheadline-emphasized: -0.0153em;

--font-footnote-regular: 400 13px/18px 'SF Pro', -apple-system, sans-serif;
--tracking-footnote-regular: -0.0062em;

--font-footnote-emphasized: 590 13px/18px 'SF Pro', -apple-system, sans-serif;
--tracking-footnote-emphasized: -0.0062em;

--font-caption-1-regular: 400 12px/16px 'SF Pro', -apple-system, sans-serif;
--tracking-caption-1-regular: 0em;

--font-caption-1-emphasized: 590 12px/16px 'SF Pro', -apple-system, sans-serif;
--tracking-caption-1-emphasized: 0em;

--font-caption-2-regular: 400 11px/13px 'SF Pro', -apple-system, sans-serif;
--tracking-caption-2-regular: 0.0055em;

--font-caption-2-emphasized: 590 11px/13px 'SF Pro', -apple-system, sans-serif;
--tracking-caption-2-emphasized: 0.0055em;
```
