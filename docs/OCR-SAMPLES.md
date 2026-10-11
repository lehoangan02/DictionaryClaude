# Handwritten note samples (reference for note reading)

The owner's own vocabulary notes, photographed on 2026-10-11. Use them to judge any change to photo reading (`crates/syncd` OCR, the Claude/agy review, a new OCR model): the text below is what a perfect read should give. Transcribed by Claude from the photos (spellings as written, including the owner's slips such as "develop abilities", "weak of"); if a line is wrong, fix it here. The photos themselves are not in the repo; ask the owner for them (page 1 = the court/woo page, page 2 = the trifling/corroborate page).

Conventions in the notes: `=` same meaning, `≈` similar (handwritten like `~~` or a wavy equals), `><` opposite, `≠` different, `sth` = something, `(v)`/`(n)` part of speech, Vietnamese glosses appear (`di căn`, `lan rộng`, `sự cân nhắc về đạo đức`).

## Page 1

```
court (v): please so to get what you want / try to get sth
woo: try to get the support of somebody
scornful of ≈ contempt ≈ disrespectful
susceptible to = easily influenced
fecklessness: not responsible, weak of character
dynamism: energy and enthusiasm to make thing happen/ succeed
punctiliousness: careful & correct behaviour / do duties properly
luminary: an expert / a person of great influence in a special area
incendiary: causing strong feelings or violence
authoritative: that you can trust as true and correct
make oneself to be: claim something about oneself
precocious: (of a child) having develop abilities early
dissemble: hide real intention
feign sth / to do sth: pretend sth / to do sth
laggardly: slow
diminutive: small
indignant: angry & surprise because you think you're treated unfairly
dilatory: slow, causing delay
```

(`court` and `try to get sth`, `dynamism … succeed` and `punctiliousness … properly` each wrap onto an indented second line; they belong to the line above.)

## Page 2

```
trifling = trivial
corroborate: support an idea with new information
belie: to hide / show something to be false
metastasize (v, t): di căn / lan rộng   ← benign / malignant tumour
indefatigable = tireless
refractory = stubborn
playful: funny and not serious
indigent = penniless = poor = needy = impecunious

imbibition ≠ osmosis
lugubriousness: sad & serious
lucubration = diligence = assiduous = sedulous
magnanimous = kind + generous (for enemies)
reprobate: a person of bad character or habits
hew to = cleave to = adhere to = conform to
piquant = interesting + exciting
bland = banal = pedestrian = hackneyed = insipid = trite
mendacious → mendacity
garrulous = talkative
accolade = praise & approval
iconoclastic >< accepted beliefs & traditions
reproach = decry = criticism = condemnation ≈ ridicule
didactic ≈ moralistic
indolence = laziness
queasiness = worried & unhappy about sth / wanting to vomit
cloying = flattery = grovel
ambivalence = mixed feeling
scruple (n): sự cân nhắc về đạo đức
harbingers: bad prophecy
improvise: 1. make do w/ what you have because you don't have what you need  2. invent, create (music, play)
```

(`metastasize` has a bracket joining `di căn` and `lan rộng`, and an arrow from `benign / malignant` and the word `tumour`; the exact structure is ambiguous. `hackneyed` is written `hackney`.)

## What the current reader does with page 1 (tesseract, v0.6.0)

It returns garbage such as `court Co) : please SO a what yor wane _= ee 4`, `Geclldecsness : not fespensible`, `Scornful gf = contempt ≈ Asrespect pull pee`, and the notes grammar then builds nonsense relations from it (`asrespect pull pee ≈ contempt`, `cimgave = ssinol`). Almost every line is unusable: cursive handwriting is outside what tesseract can read. Expected result for the same page: the 18 lines above and relations such as `scornful of ≈ contempt`, `contempt ≈ disrespectful`, `susceptible to = easily influenced`.
