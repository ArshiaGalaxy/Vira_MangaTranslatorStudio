# راهنمای مشارکت / Contributing Guide

## فارسی

**مترجم استودیو (Vira Translator Studio)** یک پروژهٔ جامعه‌محور برای جامعهٔ فارسی‌زبان است و حضورتان ارزشمند است 🙌

### چطور کمک کنم؟

1. **گزارش باگ** — در [Issues](https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio/issues) گزارش کنید و لطفاً شامل شود:
   - سیستم‌عامل، و اینکه از نصاب ویندوز استفاده کرده‌اید یا اجرا از کد
   - نسخهٔ Python/Node (فقط اگر از کد اجرا می‌کنید)
   - متن خطا (از کنسول یا لاگ برنامه)

2. **پیشنهاد قابلیت** — چیزی برای ترجمهٔ مانهوا/مانگای فارسی کم دارید؟ یک Issue با توضیح مسئله باز کنید.

3. **کد** — اول یک Issue باز کنید تا هم‌راستا شویم، بعد روی **فورک خودتان** کار کنید و Pull Request بفرستید (نه مستقیماً روی این مخزن).

### راه‌اندازی محیط توسعه

```bash
git clone https://github.com/<your-username>/Vira_MangaTranslatorStudio.git
cd Vira_MangaTranslatorStudio
git checkout -b my-change
```

**بک‌اند (Django 5 + DRF):**
```bash
cd backend
python -m venv .venv
# Windows: .venv\Scripts\activate   |   macOS/Linux: source .venv/bin/activate
pip install -r requirements-core.txt
python manage.py migrate
python manage.py runserver 127.0.0.1:8000
```

**فرانت‌اند (Next.js 15 + React 19):**
```bash
cd frontend
npm ci
npm run lint
npm run typecheck
npm run dev
```

> این پروژه در حال حاضر مجموعهٔ تست خودکار ندارد؛ لطفاً تغییرتان را به‌صورت دستی بررسی کنید (lint/typecheck + اجرای مسیر مربوطه) و در PR توضیح دهید چه چیزی را چگونه تست کرده‌اید.

ساخت نصاب دسکتاپ: [desktop/README.md](desktop/README.md).

### قوانین مهم

- ✅ طبق [LICENSE](LICENSE) (MIT) استفاده، تغییر، بازنشر و استفادهٔ تجاری آزاد است — تنها شرط، حفظ اعلان کپی‌رایت و متن لایسنس.
- 🛠 با سبک و ساختار موجود هماهنگ بمانید؛ تغییرات غیرمرتبط را در PR جداگانه بفرستید.

---

## English

**Vira Translator Studio** is a community project for Persian-speaking manga/comic translators — contributions are welcome 🙌

### How can I help?

1. **Bug reports** — open an [Issue](https://github.com/ArshiaGalaxy/Vira_MangaTranslatorStudio/issues) with your OS, whether you used the Windows installer or ran from source, Python/Node versions (source only), and the error text/logs.
2. **Feature ideas** — open an Issue describing the problem you'd like solved.
3. **Code** — open an Issue first so we can align, then work on **your own fork** and send a Pull Request (not a branch on this repo).

### Development setup

```bash
git clone https://github.com/<your-username>/Vira_MangaTranslatorStudio.git
cd Vira_MangaTranslatorStudio
git checkout -b my-change
```

**Backend (Django 5 + DRF):**
```bash
cd backend
python -m venv .venv
# Windows: .venv\Scripts\activate   |   macOS/Linux: source .venv/bin/activate
pip install -r requirements-core.txt
python manage.py migrate
python manage.py runserver 127.0.0.1:8000
```

**Frontend (Next.js 15 + React 19):**
```bash
cd frontend
npm ci
npm run lint
npm run typecheck
npm run dev
```

> This project currently has **no automated test suite**. Please verify your change manually (`npm run lint` + `npm run typecheck` + exercise the affected flow) and describe how you tested it in the PR.

Desktop installer build: [desktop/README.md](desktop/README.md).

### Ground rules

- ✅ Per the [LICENSE](LICENSE) (MIT), you may use, modify, redistribute, and use this commercially — keep the copyright notice and license text intact.
- 🛠 Match the existing style/structure and keep unrelated changes out of the PR.
