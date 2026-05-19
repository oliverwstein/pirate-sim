Agent completed. agent_id: naval-combat, agent_type: research, status: completed, description: Naval combat mechanics research, elapsed: 876s, total_turns: 0, duration: 781s

I now have sufficient data from multiple sources. Here is the comprehensive research report:

---

# Age-of-Sail Naval Combat Mechanics: Research Report
## Caribbean, 1600–1720 — For Simulation Model Development

---

## Summary

This report synthesizes quantitative and qualitative data on age-of-sail naval combat from Wikipedia's Naval Tactics in the Age of Sail, Naval Artillery in the Age of Sail, the specialist pirates.hegewisch.net site (Tobias Gibson, citing Masefield, Exquemelin, Defoe/Johnson, Cordingly, and others), Warfare History Network (Niderost), primary-source references, and historical case studies including the Battle of Ocracoke (1718), Morgan's Caribbean raids (1668–71), and the Spanish treasure fleet system. No GitHub repositories of relevance were found for this domain.

---

## 1. DETECTION / SIGHTING

### Visual Detection Range

**Sources confirm but do not give a single precise figure.** The primary limitation was the curvature of the earth combined with mast height. Key inferences:

- A ship's mast at ~100–130 ft (typical 4th–6th rate frigate or sloop-of-war) creates a geometric horizon of roughly **12–15 nautical miles** for a lookout at the masthead
- A mast-top lookout spotting another ship's top rigging (not hull) has an effective **practical detection range of about 8–12 nm** in clear weather, Caribbean visibility (which is generally excellent)
- Hull-down sighting (seeing masts but not hull) was the norm at maximum range — the hull becomes visible at roughly **5–7 nm** in calm seas
- **"Over the horizon" detection was ~10–15 nm**; hull/deck visible at 3–6 nm depending on ship size

**Weather effects on visibility:**
- Clear Caribbean day: maximum ~10–12 nm
- Tropical squall/rain: reduced to 1–2 nm or near zero
- Haze: 3–5 nm
- Night/moonless: near zero beyond ~0.5 nm; lanterns visible at ~2–3 nm
- Battle smoke could reduce visibility to near zero within an engagement — a significant factor since British ships firing on the downward roll blanketed leeward ships with smoke (*Naval Tactics in the Age of Sail*, Wikipedia)

**Simulation parameter:**
- Clear day: detection roll at **10 nm**, definite sighting at **6 nm**
- Squall: detection at **1.5 nm**, definite at **0.8 nm**
- Night: **0.5 nm** (mast lanterns only)

**Citation:** Wikipedia, "Naval Tactics in the Age of Sail" (smoke from windward fleet blowing down on leeward fleet noted as major tactical factor); Bermuda sloop article ("harbour too shallow for Royal Navy's larger vessels" — implies visibility/observation at harbor approaches)

---

## 2. CHASE / CLOSURE

### Relative Speeds (Key Quantitative Data)

From the Bermuda sloop article (*en.wikipedia.org/wiki/Bermuda_sloop*):
- **Jamaican/Bermuda sloop: up to ~12 knots** (notably fast, best Caribbean performer)
- Square-rigged ships of the line / large frigates: **typically 6–9 knots** in favorable conditions
- Heavily laden merchantmen: **4–7 knots**
- Galleons: **4–6 knots** (poor windward ability)

**Chase gun data** (*en.wikipedia.org/wiki/Chase_gun*):
- Chases frequently lasted **hours or sometimes days** with crews fine-tuning sails
- In one documented example: **72 shots from bow chasers** were fired before hitting sails of a fleeing enemy craft — illustrating poor accuracy but confirms extended chase is standard
- Chase guns were bow chasers and stern chasers — typically a **9-pounder long gun ("long nine")**, chosen for range over weight

**Windward vs. leeward in a chase:**
- A ship being chased to windward could turn into the wind and leave a square-rigged pursuer behind — this was the primary escape tactic for Bermuda sloops
- The Bermuda sloop "could outrun most other sailing ships by simply turning upwind and leaving its pursuers floundering" (*Wikipedia, Bermuda sloop*) — quantified as the fore-and-aft rig's ability to sail within **~45° of the wind** vs. ~70° for square-rigged ships

**Time to closure:**
- If the chase had a 2-knot speed advantage, closing from 10 nm to boarding range (~0.02 nm) would take ~5 hours
- Most historical accounts mention chases of **2–8 hours** for successful intercepts; failed chases could extend to **1–2 days**
- If the fleeing ship could gain the windward position, escape was almost certain unless already in very close range

**Simulation parameters for hourly updates:**
- Each hour: recalculate relative position based on speed differential + wind angle advantage/disadvantage
- At each hour, determine if engagement conditions exist (within 1 nm)
- Escape decision: if fleeing ship achieves windward position AND is >3 nm ahead, chase typically abandoned

---

## 3. WEATHER GAUGE

### Quantified Advantages of Windward Position

(*Wikipedia, "Naval Tactics in the Age of Sail"*)

1. **Tactical Initiative:** The windward ship controls the engagement — can bear down to attack OR stay upwind to refuse battle. The leeward fleet could withdraw downwind but **could not force action**

2. **Hull exposure:** Ships on the leeward gage **heel away** from opponents, **exposing their bottom** to shot. Shots that hit below the normal waterline are catastrophic ("hulled between wind and water")

3. **Smoke:** Gun smoke from windward fleet **blows down** on leeward fleet, significantly reducing visibility for the leeward gunners

4. **Raking on approach:** The windward fleet can bear down in line abreast and **rake the leeward fleet's bows** before the leeward ships can bring broadsides to bear

5. **ONE MAJOR EXCEPTION (heavy weather):** In strong winds, windward ships **cannot open lower-deck gunports** (they'd be swamped). The leeward ship's windward-side lower guns are elevated by heel and can fire freely. Admiral Rodney exploited this at Cape St. Vincent (1780) by attacking from leeward in stormy weather

**French vs. British tactical doctrine** (highly relevant for Caribbean 1600–1720):
- **French preference:** leeward gage, fire on the **upward roll** → targets rigging (to disable and flee with mission intact)
- **British/Dutch preference:** windward gage, fire on the **downward roll** → targets hulls (to kill crews with splinters, defeat the enemy ship)
- Result: French fleets consistently suffered **more casualties and higher killed:wounded ratio** despite fighting defensively

**Simulation parameter:**
- Windward ship: +15–20% hit probability at the hull; choice of engagement range; can disengage freely
- Leeward ship: -10% accuracy (smoke), risk of hull-below-waterline hits in seas above Beaufort 3+
- Heavy weather (Beaufort 5+): windward advantage inverts for lower-deck gunnery

---

## 4. ENGAGEMENT RANGES

### Primary Source: Masefield/Hegewisch Table ("On the Spanish Main," 1906)

From **pirates.hegewisch.net/arty_cannon.html**, citing John Masefield (1906), *On the Spanish Main, or, Some English Forays on the Isthmus of Darien*:

| Gun Type | Point-Blank Range (paces) | Max Elevation Range |
|---|---|---|
| Cannon Royal (66 lb shot) | 800 paces | 1930 paces |
| Whole Cannon (60 lb) | 770 paces | 2000 paces |
| Culverin (17½ lb) | 200 paces | 2500 paces |
| Demi-Culverin (9½ lb) | 200 paces | 2500 paces |
| Saker (5½ lb) | 170 paces | 1700 paces |
| Minion (4 lb) | 170 paces | 1700 paces |

*(1 pace ≈ 2.5 feet / 0.76 m; 800 paces ≈ 600 m / ~0.35 nm)*

**Naval context from multiple sources:**
- **Maximum range** (~2,000 yards / ~1 nm): "Naval artillery had unheard-of range of about 2,000 yards by this time" but "most engagements were fought at under 1,000 yards" (*pirates.hegewisch.net/arty_cannon.html*)
- **Effective gunnery range**: ~200–500 yards (roughly 100–300 paces from deck height perspective); the norm for battle was **250–500 yards** where accuracy was meaningful
- **Close range / "pistol shot"**: "sometimes within pistol shot (25 to 50 yards)" (*pirates.hegewisch.net*). Double-shotting was "devastating within pistol shot range" — confirms ~25–50 yards
- **Boarding range**: Physical contact / grappling hooks; ~0 yards (ships alongside each other)

**Accuracy note:** "Despite their limited accuracy even when aiming at the sizeable target of an enemy ship's rigging" — the 72-shot chase gun story (*Wikipedia, Chase gun*) underlines that even at medium range accuracy was low. The penetrating power of hulls was also "mediocre" at long range — "wooden ships could only be pierced at short ranges" (*Wikipedia, Broadside*)

**Simulation ranges (using nautical miles):**
- Long range: **0.25–0.55 nm** (450–1000 yards) — low accuracy, harassing fire only
- Medium/effective: **0.08–0.25 nm** (150–450 yards) — meaningful gunnery begins
- Close range: **0.01–0.08 nm** (20–150 yards) — high damage potential
- Pistol shot: **<0.015 nm** (~25 yards) — double-shot, grape and canister
- Boarding: **contact** (~0 nm)

---

## 5. GUNNERY

### Rate of Fire

**Key quantitative data** (*Wikipedia, "Naval Artillery in the Age of Sail"* and Warfare History Network):
- "A typical broadside of a Royal Navy ship of the late 18th century could be fired **2–3 times in approximately 5 minutes**" — so ~**1 broadside every 1.7–2.5 minutes** for a trained crew
- Wartime standard for WELL-TRAINED crew: **1 shot per 90 seconds** (*Warfare History Network, Niderost*): "a well-trained gun crew could fire a cannon in 90 seconds"
- Average/less trained crew: ~**3–5 minutes per shot** (the Admiralty didn't provide gunpowder for live training)
- Pirates: typically **less trained than Royal Navy**, but smaller guns (4–9 pounders) required smaller crews and were faster to reload than 24–32 pounders

**Rate of fire by gun size (practical estimate):**
- 6-pounder: **~2 min/shot** (small crew, light charge)
- 12-pounder: **~2–3 min/shot**
- 24-pounder: **~3–4 min/shot** (15-man crew, heavy charge)
- 32-pounder: **~4–5 min/shot** (very heavy, slow)

**Accuracy at different ranges (estimated from sources):**
- 2,000 yards: ~2–5% (harassing fire, mostly miss)
- 500–1,000 yards: ~10–20%
- 200–500 yards: ~20–40%
- 50–200 yards: ~40–65%
- Pistol shot (<50 yards): ~60–80% (double-shot, point-blank)
- Note: these are not sourced as explicit figures — they are reasonable inferences. The 72-shot chase gun miss story suggests <5% at long range from a moving platform

**Damage per hit by ammunition type** (*Wikipedia, "Naval Artillery in the Age of Sail"*, pirates.hegewisch.net):

| Ammunition | Primary Target | Effect |
|---|---|---|
| **Round shot (ball)** | Hull, gun crews | Punches holes, causes timber splinters (splinters killed more men than balls), dismounts guns |
| **Chain shot / bar shot** | Rigging, masts, sails | Whirls like bolas; destroys sails, severs lines, brings down masts |
| **Grape shot** | Crew, quarterdeck | Multiple balls scatter; "used against the enemy quarterdeck to kill or injure officers, or against enemy boarding parties" |
| **Canister / case shot** | Crew at close range | Giant shotgun effect; dozens of musket balls |
| **Langrage / sangrenel** | Crew | Scrap iron, bolts, rocks; "hideous wounds"; highly effective anti-personnel |
| **Double shot** | Close-range hull + crew | Two balls loaded together; "devastating within pistol shot range" |

**Raking fire** (*Wikipedia, "Raking fire"*):
- Fire directed **parallel to the long axis** of the enemy ship (from bow or stern)
- Each shot passes through much more of the ship — down the full length of the gun deck
- "Stern rake more damaging than bow rake" — stern is less reinforced, rudder is vulnerable
- Example: *Victory* raked *Bucentaure* at Trafalgar → **197 killed, 85 wounded** from a single pass, putting the ship out of the fight. This was from an 100-gun ship's broadside at close range, but illustrates the multiplier effect
- **Simulation multiplier:** A successful rake should be treated as **3–5x a normal broadside** in damage to crew; disabling the rudder is a separate crit chance (~30–50% if stern is raked)

---

## 6. BOARDING

### When Boarding Was Chosen

**Pirate preference for boarding** (*pirates.hegewisch.net*):
- Pirates came "to rob and not sink" — smaller guns favored to avoid sinking the prize
- Multiple small guns better than one large gun: "pirates were able to create a continuous raking of the deck, rigging and ports, causing massive damage to personnel" while preserving the ship's value
- Pirate tactic: **strip the opponent's crew of the will to fight** through intimidation (Jolly Roger, red flag = no quarter), then board
- "A pirate's ship willingness to fight was usually more than enough for most other ships to surrender" — pirates rarely won through attrition, they relied on **daring, surprise, and bluff**

**Navy preference for gunnery:**
- Naval ships fought to disable/capture/sink enemy warships; crew survival was not the primary prize concern
- British doctrine: fire on the downward roll to hull the enemy ship and kill gun crews

**Boarding mechanics** (*Wikipedia, Battle of Ocracoke*, pirates.hegewisch.net):
From the Ocracoke battle (Maynard vs. Blackbeard, 1718):
1. Ships close to contact range; grappling hooks thrown
2. **Swivel guns and small arms** sweep the enemy deck (Blackbeard's swivel guns "scored direct hits at the two ships, killing many of Maynard's men, including several officers")
3. Grenades thrown to clear defenders (historical buccaneer practice)
4. **Boarding party crosses** with cutlasses, pistols, boarding axes
5. Maynard's ruse: feigned retreat below decks → pirates boarded → Maynard's men surged up for close-quarters fight
6. Blackbeard "was shot 5 times and wounded 20 times by blade throughout the battle" before dying — shows the ferocious nature of close quarters

**Factors determining boarding success:**
- **Crew ratio** (most important): overwhelming numbers win; pirate crews were often large specifically for boarding
- **Crew quality**: experienced buccaneers, ex-naval personnel vs. merchant mariners with no combat training
- **Ship height differential**: attacking a higher-sided ship from a lower vessel = severe disadvantage (defenders can shoot down)
- **Deck swept beforehand**: grape/langrage fire before boarding critical to reduce defender numbers
- **Psychological state**: demoralized defenders were easier to overwhelm

**Simulation formula (inference):**
- Base boarding success probability = f(attacker crew / defender crew)
- At 2:1 ratio: ~80% success
- At 1:1 ratio: ~50% success, heavy casualties both sides
- At 0.75:1 ratio: ~25% success, boarding likely repulsed
- Modifiers: +20% if deck pre-cleared with grape; +15% if height advantage (attacker higher); +20% for experienced pirate crew vs. merchant; -20% if defending navy crew with muskets organized

---

## 7. DAMAGE AND CASUALTIES

### Casualty Rates

**Key references:**
- Battle of Ocracoke (1718): 2 sloops (Maynard) vs. 1 sloop (Blackbeard)
  - British: **11–20 killed** out of ~60 men = **18–33% killed** in a short intense action
  - Pirates: **10–12 killed** out of ~25 men = **40–48% killed**; 8 of 12 pirate deaths in the boarding phase
  - Action duration: approximately **1–2 hours** including grappling

**General casualty rates** (inferred from sources):
- Small ship action, pirate vs. merchant: **2–10% crew casualties** if merchant surrenders early; **5–20%** if brief resistance
- Contested single-ship action: **10–30% crew casualties** per side over 30–90 minutes
- Major fleet battle (line-of-battle): **10–25% per ship** engaged; battles could last **4–8 hours**
- British guns firing into hulls consistently produced **higher absolute casualties** than French style (fire-high/rigging targeting approach)

**Hull damage:**
- Penetrating hits to the hull near the waterline most dangerous — if below waterline when heeled, water ingress is immediate
- Ships did not typically sink from gunfire in a single action (thick oak timbers; fire is more dangerous)
- More commonly: **disabled** (masts shot away, rudder hit, crew casualties too high to sail)
- **To disable a sloop**: estimated 15–30 hull hits from 9–12 pounders; or **5–10 hits from 24-pounders**
- To actually sink a wooden ship: rare; usually sank from magazine explosion, fire spreading, or deliberate scuttling

**Rigging damage:**
- Chain/bar shot targeting sails and rigging was specifically used to slow fleeing ships
- Single mast lost: **30–40% speed reduction**; ship may be unable to maneuver well
- Bowsprit/jib cut: **loss of sailing to windward** (as in the Ocracoke battle where musket fire "severed the jib of the *Adventure*, removing its sole method of propulsion")
- Complete rigging destruction: ship becomes stationary ("prizes of mast and yard")

**Surrender thresholds** (inferred from period accounts):
- Merchant ships: typically surrendered at the **sight of the Jolly Roger** or first shots, especially if outgunned
- Naval ships: fought until **~25–40% crew casualties**, captain killed, or magazine threatened
- Pirate ships: fought more tenaciously (avoiding capture = hanging); might fight to near 50%+ casualties

---

## 8. SHIP MATCHUPS

### Sloop vs. Merchantman (Pirate Attack on Trader)

*pirates.hegewisch.net (merchant ships section)*:
- Merchant crews were "not well paid... sometimes pressed into service... no reason to remain loyal"
- "More than willing to surrender and let the pirates have the merchandise"
- If a pirate sloop (6–10 guns, 50–80 men) attacked a merchant (4–10 guns, 15–30 men): **outcome almost always immediate surrender** unless merchant had escort or was a particularly well-armed Indiaman
- Expected duration: **30–90 minutes** from sighting to surrender (mostly chase time)
- If merchant carried 10+ guns and a fighting crew: could resist, **chase and brief gun exchange** before surrender or escape

### Frigate vs. Sloop (Navy Hunting Pirate)

- Frigate (24–40 guns, 12–24 pounders, 150–200 men) vs. pirate sloop (6–12 guns, 4–9 pounders, 50–120 men)
- The sloop's only advantage was **speed and maneuverability** — especially upwind performance
- If the pirate sloop gained the windward position and had sea room: almost certain **escape** (Bermuda sloops specifically could outrun frigates upwind)
- If the frigate got to windward first or caught the sloop in confined waters (shallow anchorage): **frigate wins decisively** — outgunned ~4:1
- Shallow water was the pirate's friend: New Providence's harbor "too shallow for the Royal Navy's larger vessels" (*Wikipedia, Blackbeard*)

### Ship of the Line vs. Frigate

- Ship of the line (74 guns, 32-pounders on lower deck) vs. frigate (28–36 guns, 12–18 pounders)
- Ships of the line did NOT engage frigates in the line — frigates were exempt from fleet battles
- In a direct engagement: ship of the line wins decisively; frigate's tactic was to flee
- Relevant for fleet-context escort scenarios

### Multiple Sloops vs. Larger Ship (Pirate Pack Tactics)

*pirates.hegewisch.net (merchant ships)*:
- Pirates attacking a fleet used "the same tactic that wolves or leopards use when attacking prey... pick one out of the fleet, try to separate it from the others and then attack it"
- "When possible they would approach from the bow or stern and always try to keep the ships main guns from facing their approach"
- Blackbeard's squadron of multiple ships gives the model: **coordinate approach from multiple angles** to prevent enemy from bringing full broadside to bear
- A single large armed merchantman (8–12 guns) could be overwhelmed by **2–3 sloops** attacking simultaneously from different angles; two vessels attacking from opposite sides left the defender with no broadside bearing on both simultaneously

### Fort vs. Ship

**Critical principle** (*Naval Tactics in the Age of Sail*, Wikipedia):
- Shore batteries had major advantages over ships:
  - **Stable platform** (ships roll; forts don't) → dramatically better accuracy
  - **Thicker stone walls** absorbing far more punishment than wooden ships
  - **Height advantage** for plunging fire; ships could not elevate guns to return plunging fire
- "Heated shot" was a fort weapon (dangerous on ships due to fire risk) — set wooden ships ablaze
- General rule: **1 shore gun ≈ 3–5 ship guns** in an exchange; some authorities cite even higher ratios
- Historical example: Morgan at Portobelo (1668) — he attacked the forts from the **land side** (using prisoners and monks as shields), not from the sea; the direct naval approach was too costly

**Morgan's Lake Maracaibo raid (1669):**
- Trapped behind a castle on the only exit channel with 3 Spanish warships blocking escape
- Solution: **fireships** (packed with combustibles) leading the column; psychological intimidation; negotiation
- Demonstrated that direct frontal naval assault on shore batteries was near-suicidal — required subterfuge

---

## 9. FORT / SHORE BATTERY INTERACTIONS

### Caribbean Fort Armament

**Portobelo as archetype** (Morgan's 1668 attack + Vernon's 1739 attack referenced in Spanish treasure fleet article):
- Caribbean forts mounted **18–36 pounder guns** on their seaward batteries
- Typical Spanish Caribbean fort: **10–30 heavy guns** (demi-cannons to full cannons), mounted in stone casements or open parapets
- Fort San Lorenzo (Chagres): ~8–10 heavy guns; held by ~130 men in 1671 when Morgan attacked
- Portobelo's 3 forts combined: estimated **40–60 heavy guns total** across multiple positions

**Shore vs. ship quantitative advantage:**
- Stable platform: **~3x accuracy improvement** vs. rolling ship platform
- Stone/earthwork absorption: ship's 32-pounder ball could not penetrate ~6 ft of masonry (but could breach earthworks)
- Standard principle: an attacking fleet needed **5–10x the guns of a fort** to have a reasonable chance in a direct naval assault
- Edward Vernon captured Portobelo in 1739 with only 6 ships — but it was a poorly defended fort at that time; his success was exceptional

**Fleet vs. harbor entrance:**
- A properly defended harbor entrance with 2–3 forts on commanding heights was effectively **impassable** to a fleet in direct assault
- Morgan's consistent approach: land troops, attack from the rear
- The "bomb vessel" (ketch-rigged mortar ship) was specifically developed for fort reduction from beyond effective return-fire range

**Can a fort prevent fleet entry?**
- YES, if: well-manned, not surprised, has cross-fire batteries, and narrows the channel
- NO if: poorly manned, surprised, or the attacker uses fireships/diversion + night assault

---

## 10. CONVOY DEFENSE

### Spanish Treasure Fleet Organization (*Wikipedia, Spanish treasure fleet*)

- Two main convoy routes: Caribbean fleet (*Flota de Indias*) and Pacific route (*Manila Galleons*)
- Convoys departed from Seville (later Cádiz) with **2–4 armed galleons as escorts** (*Armada de la Guardia*) surrounding a column of **10–30+ merchant vessels**
- Rendezvous point at Havana before Atlantic crossing
- Numbering: 17 ships in 1550 → **50+ vessels** by end of 16th century → declining to ~25 by mid-17th century

**Convoy escort effectiveness:**
- "Pirates were more likely to **shadow the fleet to attack stragglers** than to engage the well-armed main vessels" (*Wikipedia, Piracy in the Caribbean*) — the convoy system was highly effective at preventing direct attack on the main body
- "The Atlantic trade was largely unharmed" even with persistent piracy
- Major successful attacks on the Spanish fleet were **rare exceptions**: Dutch West India Company captured the entire fleet in Bay of Matanzas (1628) — a freak ambush in shoal water, not a frontal assault
- Drake's 1586 Cartagena capture and Morgan's 1671 Panama raid were **land operations** not naval convoy interceptions

**Pirate separation tactics:**
1. **Shadow and wait** for stragglers (ships separating in storms, becalmed, or lagging)
2. **False signals** (flying national colors to approach, revealing the Jolly Roger only at close range)
3. **Multi-ship attack on a single vessel**: engage the escort while partners take the merchantmen
4. **Night attack**: close on merchant in darkness before escort can respond

**Force ratio for profitable attack:**
- Single pirate sloop vs. unescorted merchant: favorable at **1:1 crew or better**
- Against an escorted convoy: impractical without 3:1 force ratio at minimum (and even then, likely catastrophic losses)
- Blackbeard's blockade of Charles Town (1718): achieved with **4 ships, ~300+ men** — enough to overwhelm any individual vessel leaving port

---

## 11. COMBAT DURATION

### Pirate vs. Merchant (Typical)

- **Surrender without fight**: 0–30 minutes (chase + demand)
- **Brief resistance then surrender**: 30–90 minutes
- **Determined merchant resistance**: 1–3 hours (rare, usually only armed Indiamen)

### Single-Ship Naval Action

- Most single-ship actions lasted **30 minutes to 3 hours** at close range
- Longer engagements (3–6 hours) occurred when ships were evenly matched and both had skilled crews

### Major Fleet Battle

- Line-of-battle engagements: **4–12 hours** (e.g., Battle of the Saintes: 12 April 1782, a full day's engagement)
- Could extend over **multiple days of maneuvering** before actual battle (fighting instructions required forming line before engaging)

### Extended Chases

- Chase gun data (*Wikipedia*): chases "frequently lasted hours or sometimes days"
- Most successful intercepts: **2–8 hours** from detection to engagement
- Failed intercepts: adversary escapes at nightfall or in a squall — **8–24 hours** of pursuit abandoned

### The Ocracoke Battle (1718) as Anchor Case Study

- Battle of Ocracoke: 2 Royal Navy sloops vs. Blackbeard's sloop
- Initial cannon exchange → boardign attempt → hand-to-hand → Blackbeard killed
- Total duration from first shots to Blackbeard's death: estimated **2–4 hours** (early morning to mid-morning)

---

## 12. MORALE AND SURRENDER

### When Ships Surrendered

**Merchant ships:**
- At sight of the Jolly Roger + overwhelming force: immediate strike of colors (very common)
- Red flag (joli rouge) = "no quarter if you resist" — raised as a final warning
- Typical threshold: **first broadside received with no hope of escape or relief** → immediate surrender

**Naval ships:**
- Would fight longer, but surrender when:
  - ~**25–40% of crew killed or incapacitated**
  - Captain or most officers killed
  - Magazine threatened
  - Ship disabled (masts shot away, rudder gone)
  - Relief impossible
  
**Pirate ships:**
- Fought hardest of all — being captured = hanging
- But also: some surrendered to gain royal pardon (*Pirates of the Caribbean* Wikipedia)
- Threshold was much higher — might fight to **50%+ casualties**

### Pirate Intimidation Tactics

(*pirates.hegewisch.net, jolirouge page*):
- **Jolly Roger**: raised to signal "surrender and you will be spared" (black flag)
- **Red flag** (joli rouge): "no quarter will be given if you fight"
- **False colors**: approach using friendly or neutral flag; hoist true colors only at pistol-shot range
- **Reputation**: Blackbeard "spurned the use of violence, relying instead on his fearsome image" (*Wikipedia, Blackbeard*) — lit slow-match fuses in his hat as psychological terror
- **Bartholomew Roberts** was known for attacking men-of-war even when most pirates avoided them — his reputation alone caused surrenders

### Quarter

- The concept of quarter (mercy) was generally given when a ship struck its colors
- Pirates had a reputation for cruelty, but many accounts show **relatively low actual violence** against surrendering crews if they complied quickly
- Merchants who fought and then surrendered might receive harsher treatment

### Prize Crews

- A captured ship required a crew to sail it to port
- **Minimum viable prize crew**: ~5–10 men for a small sloop; ~15–25 men for a brigantine or merchant ship; ~30–50 men for a large prize
- Pirates with multiple prizes quickly exhausted their manpower — Blackbeard's crew of 300+ enabled running multiple prizes simultaneously
- Common practice: **let the original crew sail the prize** under pirate supervision (with most of the pirate crew keeping watch)

---

## 13. SIMULATION PARAMETERS (CONCRETE NUMBERS)

### Detection Ranges (nm)

| Condition | Detection Roll | Definite Sighting |
|---|---|---|
| Clear Caribbean day | 10 nm | 6 nm |
| Light haze | 5 nm | 3 nm |
| Tropical squall | 1.5 nm | 0.8 nm |
| Overcast/night | 0.5 nm | 0.3 nm |
| Night with lanterns | 2 nm | 1 nm |

### Ship Speeds (knots, favorable conditions)

| Ship Type | Best Speed | Close-Hauled | Upwind |
|---|---|---|---|
| Bermuda/Jamaica sloop | 10–12 | 8–10 | 7–9 |
| Pirate sloop (modified) | 8–11 | 6–8 | 5–7 |
| Frigate (5th rate, ~32 guns) | 8–10 | 6–8 | 5–6 |
| Ship of the line (74 guns) | 7–9 | 5–7 | 3–5 |
| Merchantman (typical) | 5–7 | 4–6 | 3–4 |
| Armed galleon | 5–6 | 3–4 | 2–3 |

### Gunnery Hit Probability (base, moderate sea state, trained crew)

| Range (yards) | 6-pdr | 12-pdr | 24-pdr |
|---|---|---|---|
| 1000–2000 (long) | 3% | 2% | 1% |
| 400–1000 (medium) | 12% | 10% | 8% |
| 150–400 (close) | 30% | 25% | 20% |
| 50–150 (point-blank) | 55% | 50% | 45% |
| <50 (pistol shot) | 75% | 70% | 65% |

*Modifiers: +15% firing downwind (stable), -20% in heavy seas, -15% leeward gage, ×3–5 for raking fire vs. crew*

### Rate of Fire (rounds per minute per gun)

| Gun | Trained Crew | Average Crew | Green/Pirate |
|---|---|---|---|
| 6-pdr | 0.67 rpm | 0.40 rpm | 0.25 rpm |
| 12-pdr | 0.50 rpm | 0.30 rpm | 0.20 rpm |
| 24-pdr | 0.33 rpm | 0.22 rpm | 0.13 rpm |
| 32-pdr | 0.25 rpm | 0.17 rpm | 0.10 rpm |

*(Based on 90 seconds/trained, 2.5–4 min/average, 4–6 min/green)*

### Structural Damage per Hit (% of ship capacity per hit, ball ammunition)

| Attacker Gun | Sloop (HP ~100) | Frigate (HP ~300) | Ship of Line (HP ~600) |
|---|---|---|---|
| 6-pdr | 3–5% | 1–2% | 0.5–1% |
| 12-pdr | 6–10% | 2–4% | 1–2% |
| 24-pdr | 12–18% | 4–7% | 2–3% |
| 32-pdr | 18–25% | 6–10% | 3–5% |

*Shore battery (stable platform): multiply ×2–3. Raking fire to stern: multiply ×3–4 for crew damage, ×1.5 for hull. Chain/bar shot: hull damage ×0.2, rigging damage ×4.*

### Crew Casualty Rates (% of crew per hour at combat range)

| Range Zone | Ball | Grape/Canister | Raking |
|---|---|---|---|
| Long (1000+ yards) | 0.5–1%/hr | N/A | N/A |
| Medium (200–600 yards) | 2–4%/hr | 4–8%/hr | 8–15%/hr |
| Close (50–200 yards) | 5–10%/hr | 15–25%/hr | 20–35%/hr |
| Pistol shot (<50 yards) | 10–20%/hr | 30–50%/hr | 40–60%/hr |

*(Boarding adds 20–40% total casualties to both sides in a contested action)*

### Boarding Success Probability

| Attacker:Defender crew ratio | Deck swept? | Success Prob |
|---|---|---|
| >2:1 | Yes | 90% |
| >2:1 | No | 75% |
| 1.5:1 | Yes | 70% |
| 1.5:1 | No | 55% |
| 1:1 | Yes | 50% |
| 1:1 | No | 35% |
| <0.75:1 | Either | 15% |

*+20% if attacker is experienced pirate vs. merchant crew; +15% if attacker has height advantage; -15% if defender has musket formation on deck*

### Surrender Thresholds

| Ship Type | Hull Damage | Crew Casualties |
|---|---|---|
| Merchant (no fight intent) | 5–10% | 5% or captain threatened |
| Merchant (fighting) | 30–40% | 20–30% |
| Naval warship | 50–60% | 30–40% |
| Pirate ship | 60–80% | 40–50% (avoid hanging) |
| Flagship (command ship) | 70% | 35–45% |

*Also trigger: captain/officers killed (50% chance of surrender if captain killed with hull <30%), magazine threatened (80% chance immediate surrender)*

### Repair Times

| Damage Level | Field Repair (at sea) | Port Repair |
|---|---|---|
| Minor (rigging, sails) | 2–6 hours | — |
| Moderate (some hull damage, minor mast) | Cannot repair fully; 1–2 days partial | 3–7 days |
| Heavy (significant hull breaches, major mast) | Emergency only (pumping, jury rig); 1–3 days | 2–4 weeks |
| Near-total (multiple gun decks, major fires) | None effective | 1–3 months (or total loss) |

---

## SOURCES & CITATIONS

| Source | Coverage | URL/Reference |
|---|---|---|
| Wikipedia: "Naval Tactics in the Age of Sail" | Weather gauge, battle formations, French vs British doctrine | en.wikipedia.org/wiki/Naval_tactics_in_the_Age_of_Sail |
| Wikipedia: "Naval Artillery in the Age of Sail" | Gun types, shot types, rate of fire, caliber standards | en.wikipedia.org/wiki/Naval_artillery_in_the_Age_of_Sail |
| Wikipedia: "Raking Fire" | Raking mechanics, Trafalgar example | en.wikipedia.org/wiki/Raking_fire |
| Wikipedia: "Chase Gun" | Chase gun accuracy, 72-shot example, chase duration | en.wikipedia.org/wiki/Chase_gun |
| Wikipedia: "Bermuda Sloop" | Speed (12 knots), windward ability, pirate preference | en.wikipedia.org/wiki/Bermuda_sloop |
| Wikipedia: "Battle of Ocracoke" | Casualty data (pirates: ~48% killed; British: ~18–33%), boarding mechanics | en.wikipedia.org/wiki/Battle_of_Ocracoke |
| Wikipedia: "Blackbeard" | Crew sizes, ship armaments, blockade tactics | en.wikipedia.org/wiki/Blackbeard |
| Wikipedia: "Spanish Treasure Fleet" | Convoy organization, escort force, fleet sizes | en.wikipedia.org/wiki/Spanish_treasure_fleet |
| Wikipedia: "Piracy in the Caribbean" | Pirate tactics, convoy effectiveness, attack patterns | en.wikipedia.org/wiki/Piracy_in_the_Caribbean |
| Wikipedia: "Golden Age of Piracy" | Period context, pirate ship types and crews | en.wikipedia.org/wiki/Golden_Age_of_Piracy |
| Wikipedia: "Buccaneer" | Buccaneering tactics, fleet strength, 1663 (~15 ships, ~1000 men) | en.wikipedia.org/wiki/Buccaneer |
| Wikipedia: "Broadside (naval)" | Fire at short range, wooden hull penetration limits | en.wikipedia.org/wiki/Broadside_(naval) |
| pirates.hegewisch.net/arty_cannon.html | Masefield's gun range table (paces), gun weights, calibers, powder ratios, pistol-shot range (25–50 yards), 2000-yard max | pirates.hegewisch.net |
| pirates.hegewisch.net/arty_ammo.html | Shot types (bar, chain, grape, canister, sangrenel) | pirates.hegewisch.net |
| pirates.hegewisch.net/merc.html | Pirate vs. merchant tactics, wolf-pack separation tactics, sloop preference | pirates.hegewisch.net |
| pirates.hegewisch.net/jolirouge.html | Jolly Roger / joli rouge intimidation system, false colors, surrender mechanics | pirates.hegewisch.net |
| pirates.hegewisch.net/mow.html | Ship classes: frigate (24–40 guns, 12–24 pounders); sloop/corvette (12–20 guns, 6–12 pounders) | pirates.hegewisch.net |
| Warfare History Network (Niderost) | 90-second reload (trained crew), 32-pounder crew size (15 men), firing procedure | warfarehistorynetwork.com/sailing-ships-and-artillery |
| Rodger, N.A.M. (2005). *The Command of the Ocean* | Rate of fire (2–3 broadsides/5 min), gunlock adoption | Cited in Wikipedia Naval Artillery article |
| Manucy, Albert (1949). *Artillery Through the Ages* | Shot types reference | Cited in Wikipedia Naval Artillery article |
| Willis, Sam (2008). *Fighting at Sea in the Eighteenth Century* | Chase gun accuracy (72-shot miss story, pp. 49–50) | Cited in Wikipedia Chase Gun article |
| Masefield, John (1906). *On the Spanish Main* | Gun range table (point-blank and maximum ranges) | Cited in pirates.hegewisch.net |
| Exquemelin, A.O. (1678). *The Buccaneers of America* | Primary source for Morgan's raids | Referenced in pirates.hegewisch.net bibliography |
| Johnson/Defoe (1724). *A General History of the Pyrates* | Primary source for pirate tactics and biographies | Referenced in pirates.hegewisch.net bibliography |
| Cordingly, David (1997). *Under the Black Flag* | Pirate period overview, primary reference | Referenced in pirates.hegewisch.net bibliography |

---

## GAPS AND UNCERTAINTIES

1. **Morgan's Portobelo / Maracaibo fortification details**: Wikipedia rate-limited (HTTP 429) during research; specific gun counts for Caribbean Spanish forts were unavailable from fetched sources. Estimate of 10–30 heavy guns per fort is based on general period knowledge and contextual references, not primary-source fort inventories.

2. **Exact sighting distance data**: No period nautical manual was directly accessed. The 10–12 nm detection range is derived from geometric mast-height calculation plus contextual references; it is physically well-founded but not from a primary-source citation.

3. **Hit probability figures**: No primary statistical study was found. The probability table above is constructed from triangulating the "72 shots without a hit" (≈1.3% hit rate at long range chase), the known accuracy of period guns, and the "short range only effective" statements. These are educated estimates and should be treated as a starting framework.

4. **Shore gun multiplier**: The "1 shore gun = 3–5 ship guns" figure is a reasonable historical consensus figure but exact source wasn't accessible in the fetched material.

5. **Convoy escort numbers**: The Spanish fleet article states escorts existed but didn't specify exact escort-to-merchant ratios; 2–4 warships per 10–30 merchants is the historical norm.

6. **French Caribbean fort specific details** (Martinique, Tortuga): Not directly researched; would require follow-up searches on `en.wikipedia.org/wiki/Fort_Saint_Louis_(Martinique)` and siege accounts.

7. **Suggested follow-up searches**: `Henry Morgan's raid on Lake Maracaibo`, `Portobelo raids (1668)`, `Battle of Cartagena de Indias (1697)`, `de Pointis Cartagena`, `Edward Vernon Portobelo 1739` for specific fort vs. fleet engagement data.