# O'zgarishlar

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versiyalash: [Semantik versiyalash](https://semver.org/lang/uz/).

«Chiqarilmagan» bo'limi chiqarishlar orasida tayyor bo'lgan ishlarni yig'adi.
Chiqarish — bu bo'limni `## [X.Y.Z] — SANA` deb qayta nomlash va `vX.Y.Z`
belgisini qo'yish: reliz tavsifi qaytadan yozilmaydi, **shu fayllardan**
yig'iladi (`scripts/release-notes.sh`).

O'zgarishlar jurnali uch tilda yuritiladi: `CHANGELOG.md` (inglizcha, asosiy),
`CHANGELOG_ru.md` va `CHANGELOG_uz.md` (shu fayl). Har bir o'zgarish uchala
faylni bitta commitda tuzatadi; versiya bo'limlarining sarlavhalari bir xil.

Versiya to'rt joyda e'lon qilinadi — `package.json`,
`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` va shu yerda;
ularni `scripts/check-versions.sh` solishtiradi.

## [0.1.2] — 2026-11-14

### Qo'shildi

- **macOS uchun Darwaza.** Bitta yig‘ma Apple Silicon’da ham, Intel’da ham ishlaydi. Dastur Developer ID sertifikati bilan imzolangan va Apple tomonidan tasdiqlangan, shuning uchun «noma’lum dasturchi» ogohlantirishisiz ochiladi: `.dmg` faylni oching va Darwaza’ni Applications papkasiga torting. Mac’da yangilanishlar Linux’dagidek keladi.
- **Nom band bo‘lsa, dastur nima qilishni so‘raydi: almashtirish, ikkalasini qoldirish yoki o‘tkazib yuborish.** Papka uchun «almashtirish» birlashtirishni bildiradi: nomi bir xil fayllar almashtiriladi, qolgani joyida qoladi. Odatiy javob — «ikkalasini qoldirish», shuning uchun Enter hech narsani ustidan yozmaydi, bir nechta moslikda esa hammasiga bir marta javob berish mumkin. Almashtiriladigan fayl avval yoniga yoziladi va faqat to‘liq yetib kelgach eskisining o‘rniga turadi: aloqa uzilishi yoki «Bekor qilish» eski faylni o‘zgarishsiz qoldiradi.
- **Ikkala fayl panelida bir nechta fayl birdaniga:** Ctrl+bosish (Mac'da ⌘) yozuv qo‘shadi, Shift+bosish oraliqni tanlaydi. Tanlanganlar birga uzatiladi, sudraladi va o‘chiriladi; o‘chirish hammasi uchun bir marta so‘raydi.
- **Terminal varag‘idagi fayllar paneli papkalarni uzatadi, jarayonni ko‘rsatadi va to‘xtatiladi.** Strelkali tugmalar aylanuvchi belgi ostida bittadan fayl nusxalardi, papkani olmasdi, nomi bir xil faylni esa ustidan yozardi. Endi tugmalar, tomonlar orasida sudrash va fayl menejeridan tashlash pastdagi uzatishlar chizig‘i orqali ishlaydi: papka butunligicha ketadi, band nom haqida esa dastur so‘raydi. Uzatish tugagach, ikkala tomon o‘zi yangilanadi.
- **Fayl panellari: yashirin fayllar, sanalar, huquqlar, saralash va yo‘lni nusxalash.** Tugma nuqta bilan boshlanadigan nomlarni yashiradi va ko‘rsatadi — ikkala fayl panelida. Qatorlarda fayl qachon o‘zgargani ko‘rinadi, yon panelda keng oynada esa `rwxr-xr-x` ham; qator ustiga olib borilsa, ikkalasi ham ko‘rinadi. Server tomonida nom bo‘yicha saralash va tanlangan narsa yoki ochiq papka yo‘lini nusxalaydigan tugma bor.
- **Papkaga havola ikki marta bosish bilan ochiladi,** masalan, havola bo‘lgan `/var/www`. Faylga havolaga bosish tegmaydi.
- **Buferda rasm bo'lsa, Ctrl+V serverdagi dasturlarda, jumladan Claude Code'da ishlaydi.** Rasm SFTP orqali serverga faqat siz o'qiy oladigan fayl bo'lib yuboriladi, dasturga esa uning yo'li qo'yiladi. Bunday dastur faqat server buferini ko'radi, shuning uchun kompyuteringizdagi rasm unga yetib bormasdi. Rasm bo'lmasa, Ctrl+V avvalgidek `^V` yuboradi; fayllar server `/tmp` ni tozalamaguncha qoladi.

### O‘zgartirildi

- **Varaqlar yonidagi yozuv faqat ulanganingizni emas, aloqa yomonligini ham ko‘rsatadi.** Server tez javob berayotganda u yashil; javob 300 ms va undan uzoq kelsa yoki bitta o‘lchovga javob bo‘lmasa — to‘q sariq; 1,5 soniyadan boshlab yoki server javob bermay qo‘ysa — qizil, unda «Server javob bermayapti…» deb yoziladi. Avval tarmoq yo‘qolganda ham ulanish butunlay uzilmaguncha yashil «Ulangan» turardi. Yomon aloqa har ikki soniyada tekshiriladi, shunda tiklangani tezroq ko‘rinadi.

### Tuzatildi

- **Bo‘sh bo‘lmagan papkani o‘chirish nima uchun bo‘lmaganini tushuntiradi,** «Fayllar» ekranida esa yaratish, qayta nomlash yoki o‘chirishdan keyingi xato ro‘yxat yangilangan zahoti yo‘qolmaydi.
- **Serverda nomi band bo‘lgan faylni yuklash endi eskisini yo‘q qilmaydi.** Serverga tashlangan fayl shu nomdagisining o‘rnini egallardi, bekor qilish yoki aloqa uzilishi esa chala yuklangan nusxani o‘chirardi — natijada eski fayl butunlay yo‘qolardi. Endi yuklash keyingi bo‘sh nomni oladi, `hisobot (2).txt`, yuklab olishda bo‘lgani kabi.
- **macOS'da dastur tugmalar birikmalari Mac'dagi barcha dasturlar kabi Cmd bilan ishlaydi:** ⌘T, ⌘W, ⌘K, ⌘F, ⌘1…9, ⇧⌘D/⇧⌘E, ⇧⌘B, varaqlar esa yana ⇧⌘[ va ⇧⌘] bilan almashadi. Ctrl qobiqqa qoldi: Ctrl+W, Ctrl+K va Ctrl+F har qanday terminaldagidek ishlaydi. Option+strelkalar yana so‘zlar bo‘ylab yuradi — terminalda ham, kiritish maydonlarida ham, varaqlar va panellar esa ⌥⌘+strelkalar bilan almashadi. ⌘W butun oynani barcha sessiyalar bilan emas, varaqni yopadi. Menyudagi maslahatlar Mac belgilarini ko‘rsatadi.
- **Claude Code to'liq ekran rejimida, tmux va vim'da sichqoncha bilan nusxalash endi buferingizga tushadi.** Bu dasturlar matnni OSC 52 buyrug'i bilan yuboradi, terminal esa uni tushunmasdi. Faqat yozish qabul qilinadi va faqat shu terminal fokusda bo'lganda: server buferingizni o'qiy olmaydi.
- **Linux'da Ctrl+Shift+C to'g'ridan-to'g'ri tizim buferiga yozadi,** nusxalash xatosi esa yo'qolib ketmay, jurnalga tushadi.
- **Sekin tarmoqda varaqni yopish, yozish va server chiqishi endi kechikish o'lchovini kutmaydi.** Varaq yonidagi `ms` boshqa hamma narsa bilan bitta navbatda o'lchanardi va yo'qotishli tarmoqda har bir o'lchov sessiyani uch soniyagacha to'xtatardi. O'lchovlar, SFTP, agent buyruqlari va portlarni yo'naltirish endi terminal bilan yonma-yon ishlaydi, javob bermayotgan server bilan xayrlashish esa bir soniyadan oshmaydi.
- **macOS'da terminalda Option Meta sifatida ishlaydi.** `Option+b` va `Option+f` — qobiqda bir so‘z orqaga va oldinga — ESC-ketma-ketlik o‘rniga maxsus belgi yuborardi, shuning uchun so‘zlar bo‘ylab harakat ishlamasdi. Buning evaziga terminal ichida Option bilan maxsus belgi kiritish (masalan, «é» uchun `Option+e`) endi ishlamaydi.
- **macOS'da Cmd+C terminalda ajratilganni nusxalaydi.** Avval buni faqat Ctrl+Shift+C va Ctrl+Insert qilardi, tizimdagi tugmalar birikmasi esa hech narsa qilmasdi: terminal ajratishni sahifadan alohida saqlaydi, shuning uchun tizimga nusxalashga narsa yo‘q edi. Hech narsa ajratilmagan bo‘lsa, bosish tizimnikicha qoladi va kiritish maydonlarida nusxalash o‘zgarmaydi. Cmd+V avval ham qo‘yardi — unga tegilmadi.
- **macOS'da terminalda bufer ishlaydi va hech narsa so‘ramaydi.** O‘qish va yozish sahifa orqali emas, tizim buferi (NSPasteboard) orqali ketadi: dasturning o‘z sxemasida `navigator.clipboard` yo ruxsat so‘raydi, yo yo‘q, rasmni esa umuman bermaydi — shu sababli buferda ekran tasviri bo‘lgan Ctrl+V serverga yetib bormasdi. TIFF ko‘rinishida nusxalangan rasm PNG ga o‘giriladi. Rasm yonidagi matn avvalgidek matn hisoblanadi: serverga faqat aniq rasm bo‘lgani yuboriladi.

### Xavfsizlik

- **Linux'da dastur jurnali va nosozlik hisobotlarini endi kompyuterning boshqa foydalanuvchilari o‘qiy olmaydi.** Papkalar o‘qish uchun ochiq edi (`0755`), jurnal fayllari ham (`0644`), jurnalda esa xost nomlari, manzillar va yo‘llar bor. Endi papkalar `0700`, fayllar `0600`, allaqachon yozilganlari ham.

## [0.1.1] — 2026-11-05

### Qo'shildi

- **Jurnallar sozlamalarda ko'rinadi** (Hisobotlar): dastur jurnali uchun yo'l va «Papkani ochish» tugmasi hamda barcha hostlar bo'yicha AI yordamchi amallari ro'yxati, rad etilganlari bilan birga.
- **Mijoz dasturi endi ochiq manbali** (GPL-3.0-or-later litsenziyasi). Manba kodi, yig'ish ko'rsatmasi va ilova tarmoq orqali yuboradigan hamma narsa ro'yxati repozitoriyda; Darwaza nomi va logotip litsenziyaga kirmaydi.
- **SFTP ekranida papkalar butunlay uzatiladi — ikkala yo'nalishda va ikki server o'rtasida to'g'ridan-to'g'ri.** Jarayon papkaning umumiy hajmi bo'yicha ko'rinadi, uzatishni bekor qilish mumkin, xatoda esa xabar to'xtagan faylni nomlaydi. Hech narsa ustiga yozilmaydi (nusxa `proj (2)` kabi bo'sh nom oladi), simvolik havolalar bo'ylab yurilmaydi va nusxalanmaydi, yarim nusxalangan fayl o'chiriladi. Serverlar o'rtasida nusxalash diskingizga tegmasdan dastur orqali oqim bilan ketadi.
- **SFTP — yon panelning doimiy bandi, o'z ekrani bilan.** Ikki panel, har birida shu kompyuter yoki ro'yxatdagi istalgan server: ikkita serverni yonma-yon ochish, papkalarni ko'rish, papka yaratish, nomini o'zgartirish, o'chirish (tasdiq bilan) va fayllarni kompyuter va server o'rtasida tugma yoki sudrab nusxalash mumkin. Hostning terminal varag'i ochiq bo'lsa, uning ulanishi ishlatiladi, aks holda alohida «faqat fayllar uchun» ulanish qilinadi (serverda qobiqsiz).
- **Yon panel sarlavhasida dasturning haqiqiy versiyasi ko'rsatiladi** — qotirilgan «v0.1» o'rniga.
- **AI yordamchi kanali (agent ruxsati) endi Windows'da ham ishlaydi.** Dastur va `darwaza-mcp` nomi tokendan hosil qilinadigan nomlangan kanal orqali muloqot qiladi: kanal nomini dasturdan oldin band qilib, tokenni qo'lga kiritib bo'lmaydi. Token har bir so'rovda baribir tekshiriladi.
- **Ro'yxatda tanlangan hostda Probel tahrirlashni ochadi** (qalam), Enter avvalgidek ulaydi; qator maslahati buni aytadi.

### O'zgardi

- **Terminalda yolg'iz Ctrl+V hech narsa qo'ymaydi:** qobiqqa faqat `^V` yuboriladi; qo'yish — Ctrl+Shift+V va Shift+Insert.
- **Dastur parollaringizni saqlaydigan nomlar o'zgardi** (tizim kalitlar ombori (keyring)dagi yozuv va master-parol tekshiruv satri endi eski ishchi nom emas, Darwaza deb ataladi). Eski yozuvlar birinchi o'qishda yoki master-parol kiritilganda o'zi ko'chiriladi; hech narsani qayta kiritish shart emas.
- **SFTP ekranida fayl ro'yxatlari va yo'l satrida shrift kattaroq,** uzun yo'l yoki nom bo'lganda o'ng panel oyna chetidan chiqib ketmaydi.
- **«Ishga tushganda oldingi varaqlarni ochish» endi odatda o'chiq.** Dastur varaqlarsiz toza ishga tushadi; sozlama yoqilsa, avvalgidek ishlaydi. Avval saqlangan varaqlar o'chirilmaydi — sozlamani yoqsangiz qaytadi.
- **Host shaklidagi AI yordamchi sozlamalari pastdagi bitta yig'iladigan bo'limga — «AI yordamchi uchun» — jamlandi, odatda yig'ilgan.** Kirish, xavf haqidagi ogohlantirishlar, faqat o'qish uchun sudo buyruqlari va yordamchi eslatmalari shu yerda; eslatmalar bloki yordamchi biror narsa yozgandagina chiqadi. Yig'ilgan bo'lim qiymatlarni saqlaydi, sarlavhadagi belgi esa kirish yoqilganini yoki eslatmalar borligini ko'rsatadi.

### Tuzatildi

- **Windows uchun yig'ish:** ssh-agent orqali kirish va agentni uzatish endi unix-soketga bog'liq emas; OpenSSH agenti ishlatiladi (`\\.\pipe\openssh-ssh-agent` yoki `SSH_AUTH_SOCK`da kanal yo'li bo'lsa, o'sha).
- **«ssh-agent mavjud emas» xabari endi faqat Unix uchun o'zgaruvchini atamaydi.** Windows'da agent `SSH_AUTH_SOCK`da emas, OpenSSH nomli kanalida turadi; maslahat endi tizimingizdagi ssh agent ishlayotganini tekshirishni so'raydi.
- **Foydalanilmayotgan hisoblarning parollari ham keyring'dagi yangi nomga ko'chadi.** Nomi o'zgargach, parol faqat o'qilganda ko'chardi; hostsiz hisob eski nom ostida abadiy qolardi. Endi birinchi ishga tushishda bitta o'tishda hammasi ko'chadi, keyring mavjud bo'lmasa — keyingi ishga tushishda takrorlanadi.
- **Yordamchi rejasini tasdiqlash oynasi to'liq sizning tilingizda.** Qadam nega taqiqlangani sababi, qadam turlarining nomlari (tekshiruv, tahrir, xizmat…) va «taqiqlangan / o'tkazib yuborilgan» sanagichlari avval interfeys tilidan qat'i nazar ruscha yoki inglizcha edi.
- **Yon panel pastidagi yozuv endi kesilmaydi.** «Lokal rejim · sinx o'chiq» rus interfeysida chetidan chiqib ketardi, endi keyingi qatorga o'tadi.
- **Agentni uzatish endi serverdagi `ssh` va `git`ni qotirmaydi.** Kompyuteringizda agent bo'lmasa (yoki `SSH_AUTH_SOCK` bo'sh joyga ishora qilsa), Darwaza uzatishni so'ramaydi, allaqachon ochilgan kanalni esa darhol yopadi — avval masofadagi buyruq hech qachon kelmaydigan javobni kutish vaqti tugaguncha kutardi.
- **Ctrl+Shift+V va Shift+Insert Linux (Wayland)da boshqa ilovalarda nusxalangan matnni qo'yadi.** Avval faqat Darwazaning o'zida nusxalangan matn qo'yilar, begona matn jimgina e'tiborsiz qolardi; endi buferni ilovaning o'zi o'qiydi, o'qish xatosi esa yo'qolib ketmay, jurnalga tushadi.
- **Host manzili, foydalanuvchi nomi va nomidagi chetki bo'sh joylar endi ulanishni buzmaydi.** « 1.1.1.1» deb saqlangan host «root@ 1.1.1.1»ga ulanib, ishlamasdi. Endi chetki bo'sh joylar saqlashda va ulanishda olib tashlanadi, avval saqlangan yoki import qilingan hostlarda ham; bo'sh qiymat va manzil ichidagi bo'sh joy esa tushunarli xato bo'ladi.
- **Mavjud hostni tahrirlash endi parolni qayta kiritishni talab qilmaydi.** Guruh yoki nomni o'zgartirish uchun parolni qayta kiritish kerak edi; endi bo'sh maydon «saqlanganini qoldirish» degani, parol faqat host yaratishda majburiy.

### Xavfsizlik

- **Windows'da agent kanali boshqa foydalanuvchilardan yopiq, token esa profil bilan ketmaydi.** Kanal faqat joriy foydalanuvchi uchun yaratiladi, `darwaza-mcp` tokenni kanal aynan sizning jarayoningiz yaratganiga ishonch hosil qilgandan keyingina yuboradi, token fayli esa `%LOCALAPPDATA%`da (ko'chma profilda emas) faqat sizga ruxsat bilan turadi.

## [0.1.0] — 2026-11-01

Darwaza'ning birinchi ommaviy chiqarilishi — butunlay sizning kompyuteringizda
ishlaydigan SSH-mijoz. Akkaunt va qurilmalar o'rtasidagi sinxronlash ixtiyoriy;
server ma'lumotlaringizni o'qiy olmaydi, ular qurilmaning o'zida shifrlanadi.

### Asosiysi

- **Akkauntsiz va tarmoqsiz ishlaydi.** Bu «bepul tarif» emas, tamoyil: hostlar,
  kalitlar, terminal, SFTP va portlarni yo'naltirish lokal ishlaydi, sinxronlash
  xatosi hech narsani to'sib qo'ymaydi.
- **Hostlar va guruhlar.** Guruh, kalit va tekshiruv holati ko'rsatilgan jadval,
  qidiruv va filtrlar. Host ikki marta bosish yoki Enter bilan ulanadi.
  `~/.ssh/config` va `known_hosts` importi; qarama-qarshi kalitlar alohida
  ko'rsatiladi.
- **Varaqlar va panellar bilan terminal.** Istalgan joylashuvda to'rttagacha
  panel: Ctrl+Shift+D o'ngga, Ctrl+Shift+E pastga bo'ladi, ajratgichlar sichqoncha
  bilan suriladi, varaqni terminal chetiga tashlash mumkin. Varaqning kontekst
  menyusi: nusxalash, nomini o'zgartirish, bo'lish, qayta ulanish, boshqalarini
  yopish. Alt+strelkalar varaqlar va panellarni almashtiradi; bufer bo'yicha
  qidiruv, chiqishdagi xatcho'plar, sessiyani asciinema formatida yozib borish,
  buyruqlar palitrasi (Ctrl+K), terminalning oltita mavzusi.
- **Nusxalash va qo'yish — kutilmagan hodisalarsiz.** Ctrl+Shift+V va
  Shift+Insert matnni butunlay va roppa-rosa bir marta qo'yadi — ko'p qatorli
  parcha o'qib ulgurmasingizdan qator-qator bajarilib ketmaydi. Ctrl+Shift+C va
  Ctrl+Insert tanlanganni nusxalaydi. Tezkor tugmalar kirill klaviatura
  tartibida ham ishlaydi.
- **Varaqlar qayta ishga tushirilgandan keyin qaytadi** — birin-ketin, parol
  so'rovlari birdaniga to'kilib kelmasligi uchun. Sozlamalarda o'chirib qo'yish
  mumkin.
- **Ulanish: parol, kalit, SSH-agent, ProxyJump, agentni uzatish, interaktiv
  autentifikatsiya.** Host kaliti birinchi ulanishda tekshiriladi; mos
  kelmasa — alohida ogohlantirish.
- **Ulanmasa, nima uchunligi aytiladi:** server ulanishni rad etdi, hisob
  ma'lumotlarini qabul qilmadi, host nomi topilmadi — interfeys tilida, dvigatelning
  xom satrlarisiz.
- **SFTP** — ikki panel, navigatsiya, fayllarni oqim bilan uzatish. Terminal
  yon panelidagi «Fayllar» tugmasi joriy sessiyaning fayllarini ochadi.
- **Portlarni yo'naltirish** — lokal, masofaviy va dinamik (SOCKS5), ulanishda
  avtomatik ishga tushish bilan.
- **Snippetlar** — `{{host}}`, `{{user}}`, `{{port}}`, `{{name}}` o'rniga qo'yish
  bilan.
- **Sirlar tizim omborida** (KWallet, GNOME Keyring), bazada parollar yo'q.
  Sozlamalardagi yangi bo'lim ular aynan qayerda saqlanishini ko'rsatadi. Ombor
  bo'lmasa, dastur parol kiritishdan oldin ogohlantiradi va master-parol yoki
  ulanishda kiritishni taklif qiladi.
- **Master-parol** (Argon2id + AES-256-GCM) va uning ustiga ikkinchi omil (TOTP).
- **Akkaunt va qurilmalar o'rtasida sinxronlash** — ixtiyoriy. Unutilgan parolni
  qog'ozdagi kalit bilan tiklash mumkin: ro'yxatdan o'tishda beriladigan 32
  belgi. Pochta orqali tiklash yo'q va hech qachon bo'lmaydi — server sizning
  nusxangizni o'qiy olmaydi, bu tizimning tuzilishi, kamchilik emas.
- **Yangilanishlarni tekshirish** — faqat «Dastur haqida»dagi tugmani bossangiz;
  dastur o'zicha tarmoqqa chiqmaydi. Yuklab olingan yangilanish imzo bilan
  tekshiriladi.
- **Interfeysning oltita tili:** uz, ru, en, kk, ky, tr. Yorug' va qorong'i
  mavzular, Darwaza palitrasi, shriftlar dastur ichida.
- **Nosozlik hisobotlari diskka.** Yig'ish sukut bo'yicha o'chiq; dastur hech
  qayerga hech narsa yubormaydi.

### AI yordamchi (agent ruxsati)

Yordamchi faqat siz unga ruxsat bergan hostlar bilan ishlaydi va o'qishdan
tashqari sizning tasdig'ingizsiz hech narsa qilmaydi.

- **Tasdiqlash bilan reja.** Yordamchi o'zini qanday atagani, qaysi sessiyadan
  ekani, server manzili, foydalanuvchi va avtomatik rad etishgacha qancha vaqt
  qolgani ko'rinadi. Qadamlar mohiyati bo'yicha belgilangan (tekshiruv, xizmat,
  fayl tahriri, oddiy buyruq), fayl tahrirlari qator-baqator farq bilan
  ko'rsatiladi. Xavfsiz amal — rad etish — fokusda turadi.
- **Taqiqlangan qadam ko'rsatiladi, lekin hech qachon bajarilmaydi.** U sabab
  bilan birga chizib qo'yiladi, bajarilish undan oldin to'xtaydi.
- **Rejaning natijasi yolg'on muvaffaqiyatsiz:** nechta qadam bajarildi, xato
  qayerda, nima taqiqlangan, nima ishga tushmadi. Natijada buyruqlar chiqishi
  yo'q — unda parollar bo'lishi mumkin. Muddati o'tgan reja belgilanadi va uni
  tasdiqlab bo'lmaydi.
- **Qaror kerak bo'lganda dastur sizni chaqiradi:** vazifalar panelidagi
  belgi yonadi va bildirishnoma keladi. Unda na buyruqlar, na host nomlari bor.
  KDE Plasma (Wayland)da bildirishnoma jiddiy bo'lib, o'zi yopilmaydi.
  Sozlamalarda o'chirib qo'yish mumkin.
- **Yordamchi nima qila oladi:** server haqida bir chaqiruvda xulosa, fayllarni
  qidirish, ishlayotgan paytda jurnallarni o'qish, katta konfiguratsiyalarni
  o'zgarishni tasvirlab tahrirlash, xizmatlarni xavfsiz qayta ishga tushirish
  (oldin konfiguratsiya, keyin holat tekshiriladi), serverdagi fayllar bilan
  ishlash, serverlar o'rtasida to'g'ridan-to'g'ri nusxalash, o'zi yopiladigan
  tunnel orqali ichki xizmatlarga kirish, sizning snippetlaringiz (sukut bo'yicha
  o'chiq alohida tumbler orqali), server haqida eslatma va avvalgi amallar
  jurnali.
- **Alohida sudo buyruqlari** yordamchiga muayyan hostda ro'yxat sifatida ruxsat
  etilishi mumkin. Ro'yxatsiz hech narsaga ruxsat yo'q.
- **Buyruq terilgan varaq.** Yordamchi o'zi qila olmaydigan narsa uchun buyruqni
  terminalga teradi, Enter'ni esa siz bosasiz.

### Xavfsizlik

- **12 belgidan qisqa master-parol qabul qilinmaydi** — tekshiruv faqat oynada
  emas, yadroda.
- **Sessiya yozuvlari va baza kompyuterning boshqa foydalanuvchilaridan
  yopilgan.** Yozuvda terminalning butun chiqishi bor — konfiguratsiyalar,
  tokenlar, parollarning aks-sadosi. Ma'lumotlar va yozuvlar katalogi — `0700`,
  fayllar — `0600`.
- **Akkaunt parolini almashtirish sinxronlangan ma'lumotlarga xavf solmaydi.**
  Yozuvlar alohida kalit bilan shifrlanadi, parol uni faqat ochadi.
- **Tasdiqlangan rejaning qadami o'rtasida uzilmaydi** (15 daqiqagacha), server
  yarim o'zgargan holda qolmasligi uchun; faqat o'qiydigan buyruq 30 soniyadan
  keyin uziladi.
- **Yordamchining hostga ruxsatini yoqish alohida tasdiqni talab qiladi;**
  o'chirish darhol bajariladi.

### Asos

Hammasi nimadan boshlangani (birinchi ishlaydigan versiya, 2026-yil may):

- Hostlar va guruhlar: yaratish, tahrirlash, o'chirish, qidirish, `~/.ssh/config`
  importi.
- Parol yoki shaxsiy kalit bilan SSH ulanishi, server PTY'si, bir vaqtda bir
  nechta sessiya, keep-alive.
- xterm.js'dagi terminal: varaqlar, bufer bo'yicha qidiruv, nusxalash,
  havolalar, shrift sozlamalari.
- TOFU bo'yicha ma'lum hostlar: birinchi ulanishda tasdiqlash, kalit mos
  kelmasa alohida ogohlantirish, boshqaruv sahifasi.
- Sirlar tizim keyring'ida — bazada parollar yo'q.
- Interfeys tillari: uz, ru, en.

<!-- Taqqoslash va reliz havolalari loyiha manzili ma'lum bo'lganda shu yerda
     paydo bo'ladi. Uni oldindan o'ylab topish mumkin emas: reliz tavsifidagi
     noto'g'ri havola yo'qdan battar — odamlar u orqali ketadi va qaytmaydi. -->
