# iOS 26 Materials & Liquid Glass Specification

> Источник: Figma Canvas `Materials` (Node ID: `215:105157`), файл `omHrdST5YSkR9on4fczusp` (iOS and iPadOS 26 Community).  
> Все числовые параметры приведены без округлений и модификаций. Сырые данные сохранены в `docs/figma/raw/materials_215_105157.txt`.

---

## 1. Системные материалы (`_Materials/Mode Options`, ID: `510:78330`)

Размеры плиток материалов: `100 x 100 px` (для Chrome в Light: `width: 100px`, vertical: `hug`).

### 1.1. Сводная таблица параметров материалов

| Материал | Тема / Режим | Node ID | Background Blur | Заливки (Fills) | Эффекты |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Ultrathin** | Light | `510:78329` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.03)`<br>`rgba(255, 255, 255, 0.07)` | backdropFilter: `blur(50px)` |
| **Ultrathin** | Dark | `510:78347` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.02)` | backdropFilter: `blur(50px)` |
| **Ultrathin** | IC - Light | `3095:22074` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.03)`<br>`rgba(255, 255, 255, 0.07)` | backdropFilter: `blur(50px)` |
| **Ultrathin** | IC - Dark | `3095:22069` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.02)` | backdropFilter: `blur(50px)` |
| **Thin** | Light | `510:78331` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.4)`<br>`rgba(255, 255, 255, 0.05)` | backdropFilter: `blur(50px)` |
| **Thin** | Dark | `510:78345` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.26)` | backdropFilter: `blur(50px)` |
| **Thin** | IC - Light | `3095:22073` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.4)`<br>`rgba(255, 255, 255, 0.05)` | backdropFilter: `blur(50px)` |
| **Thin** | IC - Dark | `3095:22068` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.26)` | backdropFilter: `blur(50px)` |
| **Regular** | Light | `510:78333` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.6)`<br>`rgba(255, 255, 255, 0.25)` | backdropFilter: `blur(50px)` |
| **Regular** | Dark | `510:78343` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.41)` | backdropFilter: `blur(50px)` |
| **Regular** | IC - Light | `3095:22072` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.6)`<br>`rgba(255, 255, 255, 0.25)` | backdropFilter: `blur(50px)` |
| **Regular** | IC - Dark | `3095:22067` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.41)` | backdropFilter: `blur(50px)` |
| **Thick** | Light | `510:78335` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.84)`<br>`rgba(255, 255, 255, 0.34)` | backdropFilter: `blur(50px)` |
| **Thick** | Dark | `510:78341` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.6)` | backdropFilter: `blur(50px)` |
| **Thick** | IC - Light | `3095:22071` | `50px` (`blur(50px)`) | `rgba(255, 255, 255, 0.84)`<br>`rgba(255, 255, 255, 0.34)` | backdropFilter: `blur(50px)` |
| **Thick** | IC - Dark | `3095:22066` | `50px` (`blur(50px)`) | `rgba(0, 0, 0, 0.6)` | backdropFilter: `blur(50px)` |
| **Chrome** | Light | `510:78337` | `25px` (`blur(25px)`) | `rgba(255, 255, 255, 0.75)` | backdropFilter: `blur(25px)` |
| **Chrome** | Dark | `510:78339` | `50px` (`blur(50px)`) | `#7C7C7C`<br>`rgba(28, 28, 28, 0.9)` | backdropFilter: `blur(50px)` |
| **Chrome** | IC - Light | `3095:22070` | `25px` (`blur(25px)`) | `rgba(255, 255, 255, 0.75)` | backdropFilter: `blur(25px)` |
| **Chrome** | IC - Dark | `3095:22065` | `50px` (`blur(50px)`) | `#7C7C7C`<br>`rgba(28, 28, 28, 0.9)` | backdropFilter: `blur(50px)` |

---

## 2. Стиль iOS 26 Liquid Glass

Компоненты Liquid Glass построены из комбинации слоев: базовый стеклянный контейнер (`Glass Effect`) со скруглением и подложкой/тенью (`Fill + Shadow` или `Tint + Shadow`).

### 2.1. Liquid Glass - Regular - Large (`5584:32504`)
* **Базовые размеры:** `160 x 160 px` (в экранах/контейнерах растягивается, например, `585 x 300 px`).
* **Скругление углов (`borderRadius`):** `34px`.

#### Вариант: Dark (`5584:31860`)
* **Слой `Fill + Shadow` (ID: `5607:25707`):**
  * `borderRadius`: `34px`
  * `opacity`: `0.6700000166893005`
  * `fills`:
    1. `rgba(255, 255, 255, 0.06)`
    2. `rgba(0, 0, 0, 0.8)`
    3. `#CCCCCC`
  * `effects` (Box Shadow):
    * `offset-x`: `0px`
    * `offset-y`: `8px`
    * `blur-radius`: `40px`
    * `spread`: `0px`
    * `color`: `rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5607:25699`):**
  * `borderRadius`: `34px`

#### Вариант: Light (`5584:31859`)
* **Слой `Fill + Shadow` (ID: `5607:25683`):**
  * `borderRadius`: `34px`
  * `fills`:
    1. `rgba(250, 250, 250, 0.7)`
    2. `#262626`
  * `effects` (Box Shadow):
    * `offset-x`: `0px`
    * `offset-y`: `8px`
    * `blur-radius`: `40px`
    * `spread`: `0px`
    * `color`: `rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5607:25691`):**
  * `borderRadius`: `34px`

---

### 2.2. Liquid Glass - Regular - Medium (`5584:32503`)
* **Базовые размеры:** `160 x 160 px` (в контейнерах `585 x 180 px`).
* **Скругление углов (`borderRadius`):** `34px`.

#### Вариант: Dark (`5584:31858`)
* **Слой `Fill + Shadow` (ID: `5607:25810`):**
  * `borderRadius`: `34px`
  * `opacity`: `0.6700000166893005`
  * `fills`:
    1. `rgba(255, 255, 255, 0.06)`
    2. `rgba(0, 0, 0, 0.6)`
    3. `#CCCCCC`
  * `effects` (Box Shadow):
    * `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5607:25716`):**
  * `borderRadius`: `34px`

#### Вариант: Light (`5584:31857`)
* **Слой `Fill + Shadow` (ID: `5607:25956`):**
  * `borderRadius`: `34px`
  * `fills`:
    1. `rgba(245, 245, 245, 0.6)`
    2. `#262626`
  * `effects` (Box Shadow):
    * `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5607:25843`):**
  * `borderRadius`: `34px`

---

### 2.3. Liquid Glass - Regular - Small (`5584:32505`)
* **Размеры:** `48 x 48 px`.

#### Вариант: Dark, State=Default (`5426:6767`)
* **Слой `Fill + Shadow` (ID: `5607:79976`):**
  * `borderRadius`: `1000px`
  * `opacity`: `0.6700000166893005`
  * `fills`:
    1. `rgba(255, 255, 255, 0.06)`
    2. `rgba(0, 0, 0, 0.6)`
    3. `rgba(204, 204, 204, 0.5)`
  * `effects` (Box Shadow):
    * `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5607:78589`):**
  * `borderRadius`: `296px`
  * `fills`: `rgba(0, 0, 0, 0.2)`

#### Вариант: Dark, State=Primary (`5431:1764`)
* **Слой `Tint + Shadow` (ID: `5609:49572`):**
  * `borderRadius`: `1000px`
  * `fills`:
    1. `#0091FF`
    2. `#999999`
    3. `#FFFFFF`
    4. `rgba(255, 255, 255, 0.75)`
  * `effects` (Box Shadow):
    * `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5609:49417`):**
  * `borderRadius`: `296px`

#### Вариант: Light, State=Default (`5564:42811`)
* **Слой `Fill + Shadow` (ID: `6491:20172`):**
  * `borderRadius`: `296px`
  * `fills`:
    1. `#F7F7F7`
    2. `#DDDDDD`
    3. `rgba(255, 255, 255, 0.65)`
  * `effects` (Box Shadow):
    * `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `6491:20173`):**
  * `borderRadius`: `296px`

#### Вариант: Light, State=Primary (`5564:42807`)
* **Слой `Tint + Shadow` (ID: `5607:78140`):**
  * `borderRadius`: `1000px`
  * `fills`:
    1. `#0091FF`
    2. `#999999`
    3. `#FFFFFF`
    4. `rgba(255, 255, 255, 0.75)`
  * `effects` (Box Shadow):
    * `0px 8px 40px 0px rgba(0, 0, 0, 0.12)`
* **Слой `Glass Effect` (ID: `5607:77747`):**
  * `borderRadius`: `296px`

---

### 2.4. Liquid Glass - Clear/Light (`5914:23547`)
* **Размеры:** `160 x 160 px`
* **Слой `Glass Effect` (ID: `5914:23552`):**
  * `borderRadius`: `24px`
  * `fills`: `rgba(255, 255, 255, 0.07)`
  * `effects` (Box Shadow):
    * `offset-x`: `0px`
    * `offset-y`: `8px`
    * `blur-radius`: `40px`
    * `spread`: `0px`
    * `color`: `rgba(0, 0, 0, 0.2)`

---

## 3. Эффекты края прокрутки (Scroll Edge Effects)

### 3.1. Scroll Edge Effect - Soft (`5661:26943`)
* **Размеры:** `200 x 200 px`
* **Backdrop Filter Blur:** `blur(5px)`
* **Градиентные маски (`Gradient Mask`):**
  * **Edge=Leading (`5661:26942`):**  
    `linear-gradient(90deg, rgba(255, 0, 0, 0.9) 0%, rgba(255, 0, 0, 0) 100%)`
  * **Edge=Trailing (`5661:26944`):**  
    `linear-gradient(270deg, rgba(255, 0, 0, 0.9) 0%, rgba(255, 0, 0, 0) 100%)`
  * **Edge=Bottom (`5661:26968`):**  
    `linear-gradient(0deg, rgba(255, 0, 0, 0.9) 0%, rgba(255, 0, 0, 0) 100%)`
  * **Edge=Top (`5661:26964`):**  
    `linear-gradient(180deg, rgba(0, 0, 0, 1) 0%, rgba(0, 0, 0, 0) 100%)`

### 3.2. Scroll Edge Effect - Hard (`5661:26978`)
* **Размеры:** `200 x 200 px`
* **Backdrop Filter Blur:** `blur(30px)`
* **Заливка (Fill):** `#FFFFFF`
* **Обводка (Stroke):** `rgba(0, 0, 0, 0.12)`
* **Параметры обводки по граням:**
  * **Edge=Bottom (`5661:26984`):** `strokeWeight: 1px 0px 0px 0px` (только верхний край)
  * **Edge=Top (`5661:26986`):** `strokeWeight: 0px 0px 1px 0px` (только нижний край)
