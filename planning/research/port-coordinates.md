# Port Coordinates: Historical Anchorages c. 1680
## Research Document for pirate-sim `data/registries/ports.ron`

*Compiled from: Wikipedia (geographic coordinates), cross-referenced with*
*English Pilot, Fourth Book (1671–1745 eds.) and NGA Sailing Directions Pub. 147*
*(Caribbean) and Pub. 140 (West Africa) for historical anchorage offsets.*

---

## 1. Coordinate Convention

The simulator uses an equirectangular projection anchored at **17.5°N, 72.5°W**
with units in nautical miles (NM):

```
x_nm = (lon_decimal + 72.5) × 60   [east is positive; lon is negative for Western Hemisphere]
y_nm = (lat_decimal − 17.5) × 60   [north is positive]
```

**Worked example — Port Royal, Jamaica:**
- Wikipedia: 17°56′15″N 76°50′28″W = 17.9375°N, 76.8411°W
- x = (−76.8411 + 72.5) × 60 = −4.3411 × 60 = **−260.5**
- y = (17.9375 − 17.5) × 60 = 0.4375 × 60 = **+26.3**

All coordinates WGS-84 decimal degrees. "Anchorage position" means the open
roadstead or outer harbour where ocean-going vessels (50–400 t) moored c. 1680,
not the modern city centroid or inner dock. River/estuary ports are placed at
the river mouth or bar since deep-draft ships could not proceed further inland.

---

## 2. Port Entries

### 2.1 Spanish Ports

---

#### Havana (*La Habana*)
| Field | Value |
|---|---|
| WGS-84 | 23.1450°N, 82.3600°W |
| x_nm / y_nm | **−591.6 / +338.7** |
| Current ports.ron | (−592.8, 337.8) → Δ ~1.5 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Havana harbour is the safest anchorage on Cuba's north coast,
used continuously by Spanish plate fleets since 1519. The inner harbour
(*ensenada*) behind Castillo de los Tres Reyes del Morro is the anchorage;
entry is through a narrow bocana at approximately 23.145°N, 82.360°W. Spanish
treasure galleons anchored in the inner basin off Habana Vieja. The city-centre
Wikipedia coordinate (23.13667°N, 82.35889°W) sits inside the inner harbour and
is a valid anchorage position; proposed value is 1 NM NNW to the bocana
approaches where large ships waited for tide/wind.

**Sources:** Wikipedia "Havana" (23°08′12″N 82°21′32″W city centre, inner bay);
Castillo del Morro coordinates cross-checked via Google Maps.

---

#### Portobelo
| Field | Value |
|---|---|
| WGS-84 | 9.5544°N, 79.6550°W |
| x_nm / y_nm | **−429.3 / −476.7** |
| Current ports.ron | (−429.0, −477.0) → Δ ~0.4 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Wikipedia exact. Portobelo's inner harbour (Bahía de Portobelo)
is a deep, enclosed bay; the anchorage is the bay itself at approximately the
town coordinates. Portobelo was the terminus of the Camino Real from Panama City
and the site of the Spanish South Seas trade fairs through 1739. In 1680 it was
the most important entrepôt on the Darien coast.

**Sources:** Wikipedia "Portobelo, Colón" (9°33′16″N 79°39′18″W).

---

#### Cartagena de Indias
| Field | Value |
|---|---|
| WGS-84 | 10.2500°N, 75.5667°W |
| x_nm / y_nm | **−184.0 / −435.0** |
| Current ports.ron | (−181.8, −426.0) → Δ **~9 NM** |
| Status | ⚠️ **Significant correction** |

**Rationale:** The city of Cartagena sits inside the Bahía de Cartagena, behind
the Tierra Bomba island fortifications. Deep-draft vessels in 1680 could not
safely enter through the *Boca Chica* (southern entrance, the main channel) in
adverse conditions and often anchored in the outer roads south of Boca Chica at
approximately 10.25°N, 75.57°W. Wikipedia gives the city centre at
10.400°N, 75.500°W, but this is inside the inner harbour; the outer roads/Boca
Chica approach at 10.25°N is the historically correct open anchorage. Current
ports.ron position (−181.8, −426.0) = ~10.3°N, 75.53°W is in the inner bay; the
proposed value moves it ~9 NM SSW to the outer roads.

**Sources:** Wikipedia "Cartagena, Colombia" (10°24′N 75°32′W city);
NGA Pub. 147 Vol. I §Colombia for outer anchorage.

---

#### Santo Domingo
| Field | Value |
|---|---|
| WGS-84 | 18.4667°N, 69.9000°W |
| x_nm / y_nm | **+156.0 / +58.0** |
| Current ports.ron | (157.2, 58.2) → Δ ~1.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Santo Domingo (now the capital of the Dominican Republic) was
founded in 1498 at the mouth of the Río Ozama. The anchorage c. 1680 was in the
roadstead off the river mouth at approximately 18.47°N, 69.90°W. Wikipedia
city-centre coordinate is essentially correct for the outer anchorage.

**Sources:** Wikipedia "Santo Domingo" (18°28′N 69°54′W).

---

#### Santiago de Cuba
| Field | Value |
|---|---|
| WGS-84 | 19.9833°N, 75.8833°W |
| x_nm / y_nm | **−203.0 / +149.0** |
| Current ports.ron | (−199.2, 151.2) → Δ **~4 NM** |
| Status | ⚠️ Minor correction |

**Rationale:** Santiago de Cuba is at the head of a deep, fjord-like harbour.
The harbour entrance channel is at approximately 19.983°N, 75.883°W (Castillo
del Morro on the eastern side). This is ~4 NM SW of the city centre; ships
entering/leaving would pass through here. Wikipedia gives city centre at
20.022°N, 75.829°W (inside the harbour basin). The entrance approach is the
natural deep-water anchorage/roads position.

**Sources:** Wikipedia "Santiago de Cuba" (20°01′18″N 75°49′46″W city centre).

---

#### San Juan, Puerto Rico
| Field | Value |
|---|---|
| WGS-84 | 18.4667°N, 66.1167°W |
| x_nm / y_nm | **+383.0 / +58.0** |
| Current ports.ron | (382.8, 58.2) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** San Juan harbour is the sheltered bay (*Bahía de San Juan*) on
the north coast of Puerto Rico. The outer harbour entrance east of Castillo San
Felipe del Morro is approximately 18.47°N, 66.12°W. Current value is exact.

**Sources:** Wikipedia "San Juan, Puerto Rico" (18°27′N 66°4′W).

---

#### Maracaibo
| Field | Value |
|---|---|
| WGS-84 | 10.9833°N, 71.6000°W |
| x_nm / y_nm | **+54.0 / −391.0** |
| Current ports.ron | (52.2, −412.2) → Δ **~21 NM north** |
| Status | ⚠️ **Significant correction** |

**Rationale:** Lake Maracaibo is connected to the Gulf of Venezuela through
a shallow strait (the *Barra de Maracaibo*) guarded by Castillo San Carlos de
la Barra at approximately 11.0°N, 71.6°W. Ocean-going vessels of any draft
could not proceed past the bar into the lake without lightening; the 17th-century
anchorage was at the bar entrance. Henry Morgan's 1669 raid on Maracaibo used
exactly this choke point: he was bottled in by three Spanish frigates placed
across the channel. The current ports.ron position (52.2, −412.2) = ~10.63°N,
71.63°W is deep in the lake near the city; historically correct is the bar/narrows
at ~11.0°N, 71.6°W. The harbor_radius_nm: 30.0 in the current entry accounts
for navigating to the actual settlement inside the lake; this radius should be
preserved (or widened to 35.0 if navmesh tiles fail to connect bar to city).

**Sources:** Wikipedia "Lake Maracaibo" (lat/lon of narrows);
Wikipedia "Henry Morgan" (raid account, 1669 bar battle).

---

#### La Guaira
| Field | Value |
|---|---|
| WGS-84 | 10.6000°N, 66.9331°W |
| x_nm / y_nm | **+334.0 / −414.0** |
| Current ports.ron | (334.2, −414.0) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** La Guaira is the port of Caracas, on Venezuela's northern
coast below a steep coastal range. The anchorage is an open roadstead directly
off the town at approximately 10.60°N, 66.93°W. Wikipedia coordinate is exact.
This was a significant Spanish colonial port and the principal outlet for cacao
exports from Caracas province.

**Sources:** Wikipedia "La Guaira" (10°36′N 66°56′W).

---

#### Trinidad (*Puerto España / Scarborough, Tobago ≠ Trinidad*)
| Field | Value |
|---|---|
| WGS-84 | 10.6500°N, 61.5167°W |
| x_nm / y_nm | **+659.0 / −411.0** |
| Current ports.ron | (658.8, −411.0) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** The port of Trinidad in 1680 was the Spanish settlement at
*Puerto de España* (modern Port of Spain) on the Gulf of Paria's western
shore. The anchorage is the gulf roadstead at approximately 10.65°N, 61.52°W.
Wikipedia gives Port of Spain at 10.652°N, 61.518°W — essentially exact.
Note: Port of Spain sits inside the Gulf of Paria, which is sheltered from
Atlantic swell; ocean ships entered via the Boca del Dragón (north) or
Serpent's Mouth (south).

**Sources:** Wikipedia "Port of Spain" (10°39′N 61°31′W).

---

#### Margarita (*Pampatar*)
| Field | Value |
|---|---|
| WGS-84 | 10.9994°N, 63.7944°W |
| x_nm / y_nm | **+522.3 / −390.0** |
| Current ports.ron | (517.8, −390.0) → Δ **~4.5 NM east** |
| Status | ⚠️ Minor correction |

**Rationale:** Margarita Island's principal harbour was **Pampatar** on the
northeastern coast, protected by Castillo San Carlos Borromeo. The western-coast
settlement of La Asunción and the bay of Juan Griego also existed, but Pampatar
was the main port of entry. Wikipedia gives Pampatar at 10°59′58″N 63°47′40″W
= 10.9994°N, 63.7944°W, which is ~4.5 NM east of the current ports.ron position
(which corresponds to ~11.0°N, 64.13°W, roughly mid-island open water).

**Sources:** Wikipedia "Pampatar" (10°59′58″N 63°47′40″W).

---

### 2.2 English Ports

---

#### Port Royal, Jamaica
| Field | Value |
|---|---|
| WGS-84 | 17.9375°N, 76.8411°W |
| x_nm / y_nm | **−260.5 / +26.3** |
| Current ports.ron | (−260.4, 26.4) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Port Royal occupied the tip of the Palisadoes sandbar at the
entrance to Kingston Harbour. In 1680 it was the premier English entrepôt in
the Caribbean and the privateering capital of the West Indies. The anchorage
was the harbour immediately inside the Palisadoes at the coordinates above.
Wikipedia is exact. **Note:** Port Royal was destroyed by the earthquake of
7 June 1692; Kingston was founded as a replacement. See open questions regarding
the Kingston entry below.

**Sources:** Wikipedia "Port Royal, Jamaica" (17°56′15″N 76°50′28″W).

---

#### Kingston ⚠️ ANACHRONISM
| Field | Value |
|---|---|
| WGS-84 | 17.9375°N, 76.8411°W (proposed = Port Royal anchorage) |
| x_nm / y_nm | **−260.5 / +26.3** |
| Current ports.ron | (−257.4, 28.2) → Δ ~4 NM |
| Status | 🚨 **See Open Questions §4.1** |

**Rationale:** Kingston was **founded in 1692–1693** as a direct replacement
for Port Royal following the earthquake. It does not exist in 1680. The current
ports.ron entry has Kingston placed ~4 NM northeast of Port Royal inside
Kingston Harbour — this is geographically reasonable for the post-earthquake
city but chronologically wrong for a 1680 scenario. Recommended action: either
**remove the Kingston entry entirely** (Port Royal covers the same harbour) or
retain it with Port Royal's coordinates (same anchorage) as a duplicate to
preserve any code that references "Kingston" by name.

**Sources:** Wikipedia "Kingston, Jamaica" (founded 1692);
Wikipedia "Port Royal, Jamaica" (1692 earthquake).

---

#### Bridgetown, Barbados
| Field | Value |
|---|---|
| WGS-84 | 13.1000°N, 59.6333°W |
| x_nm / y_nm | **+772.0 / −264.0** |
| Current ports.ron | (772.8, −264.0) → Δ ~0.8 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Bridgetown's roadstead (*Carlisle Bay*) on the southwest coast
of Barbados was the principal English anchorage in the Lesser Antilles. The
open roadstead in Carlisle Bay is at approximately 13.10°N, 59.63°W; Wikipedia
gives the city at 13°06′N 59°37′W = 13.1°N, 59.617°W. Current value is accurate.
Barbados was the most populous English Caribbean colony in 1680 and a major
sugar producer.

**Sources:** Wikipedia "Bridgetown" (13°06′N 59°37′W).

---

#### Basseterre, St. Kitts
| Field | Value |
|---|---|
| WGS-84 | 17.3000°N, 62.7167°W |
| x_nm / y_nm | **+587.0 / −12.0** |
| Current ports.ron | (586.8, −12.0) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Basseterre Road (the open anchorage off Basseterre) is on the
SW coast of St. Kitts. Wikipedia gives 17°18′N 62°43′W = 17.30°N, 62.717°W.
St. Kitts (then St. Christopher) was England's first Caribbean colony (1623)
and the most important English island in the Leewards in 1680.

**Sources:** Wikipedia "Basseterre" (17°18′N 62°43′W).

---

#### English Harbour, Antigua
| Field | Value |
|---|---|
| WGS-84 | 17.0000°N, 61.7667°W |
| x_nm / y_nm | **+644.0 / −30.0** |
| Current ports.ron | (644.4, −30.0) → Δ ~0.4 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** English Harbour (*Freeman's Bay* / *Falmouth Harbour*) is a
deep natural harbour on the south coast of Antigua. It became the Royal Navy's
principal Caribbean base in the 18th century but was already in use as an
anchorage in the 1670s–80s. Wikipedia gives 17°00′N 61°46′W. The harbour
entrance is at approximately 17.00°N, 61.77°W.

**Sources:** Wikipedia "English Harbour, Antigua" (17°00′N 61°46′W).

---

#### Charleston (*Charles Town*), South Carolina
| Field | Value |
|---|---|
| WGS-84 | 32.7333°N, 79.8833°W |
| x_nm / y_nm | **−443.0 / +914.0** |
| Current ports.ron | (−445.8, 916.8) → Δ **~4 NM** |
| Status | ⚠️ Minor correction |

**Rationale:** Charles Town was founded in 1670 on a peninsula at the
confluence of the Ashley and Cooper Rivers. The anchorage for ocean ships was
off the harbour bar at the mouth of the estuary, approximately 32.73°N, 79.88°W.
Wikipedia gives the city at 32.776°N, 79.931°W (inner harbour); the bar anchorage
is ~4 NM SSE. In 1680 Charles Town had fewer than 2,000 settlers but was the
primary port for the Carolina colony.

**Sources:** Wikipedia "Charleston, South Carolina" (32°46′N 79°57′W).

---

#### Boston, Massachusetts
| Field | Value |
|---|---|
| WGS-84 | 42.3167°N, 70.9500°W |
| x_nm / y_nm | **+93.0 / +1489.0** |
| Current ports.ron | (86.4, 1491.6) → Δ **~7 NM east** |
| Status | ⚠️ Minor correction |

**Rationale:** Boston harbour in 1680 was approached through *Nantasket Roads*
(now Hull Roads), the deep-water outer roadstead where ocean ships waited for
pilots at approximately 42.32°N, 70.95°W. The inner harbour anchorage off Long
Wharf was inside (42.36°N, 71.05°W) but only accessible after threading the
harbour islands. The King's Road/Nantasket approach position is the historically
correct outer anchorage. Current value corresponds to approximately 42.34°N,
71.02°W (inner harbour approaches); the proposed position moves to the outer
roadstead ~7 NM to the east.

**Sources:** Wikipedia "Boston" (42°21′N 71°4′W city centre).

---

#### New York (*New Orange / New York*)
| Field | Value |
|---|---|
| WGS-84 | 40.6833°N, 74.0167°W |
| x_nm / y_nm | **−91.0 / +1391.0** |
| Current ports.ron | (−90.6, 1392.6) → Δ ~1.8 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** New York (retaken by England from the Dutch in 1674) had its
anchorage in the Upper Bay (*Buttermilk Channel*/*East River*). The outer
anchorage was in the Narrows at approximately 40.61°N, 74.05°W, with inner
anchorage off the town at 40.70°N, 74.01°W. The current position and proposed
position are both within reasonable range; harbor_radius_nm: 60.0 covers the
harbour and lower Hudson, which is appropriate. No significant change needed.

**Sources:** Wikipedia "New York City" (40°42′N 74°00′W).

---

#### Philadelphia, Pennsylvania ⚠️ SIGNIFICANT CORRECTION
| Field | Value |
|---|---|
| WGS-84 | 38.9000°N, 75.1000°W |
| x_nm / y_nm | **−156.0 / +1284.0** |
| Current ports.ron | (−177.8, 1312.6) → Δ **~38 NM** |
| Status | 🚨 **Significant correction** |

**Rationale:** Philadelphia was founded in 1682 (just after the sim's 1680
cutoff) on the Delaware River at approximately 39.95°N, 75.15°W — over 100 NM
upstream from the Delaware Bay mouth. Ocean-going ships anchored in Delaware
Bay, not in the river. The outer Delaware Bay anchorage off Cape Henlopen
(38.78°N, 75.08°W) was the standard waiting/waypoint position for ships bound
for Philadelphia. The current ports.ron comment already acknowledges "anchor at
bay mouth," but the coordinate (−177.8, 1312.6) = ~39.38°N, 75.46°W places the
sim port approximately in the middle of the Delaware Bay near Wilmington, DE —
not at the sea approach. The corrected position is the outer Delaware Bay
approaches at Cape Henlopen. **Note:** Philadelphia barely exists in 1680 (Penn's
Charter was 1681); it is the quintessential 1690s–1720s colonial port. Consider
whether it should be present in a strict 1680 scenario or treated as a late-start
port.

**Sources:** Wikipedia "Philadelphia" (39°57′N 75°10′W city);
Wikipedia "Cape Henlopen" (38°47′N 75°05′W).

---

#### Bermuda (*St. George's*)
| Field | Value |
|---|---|
| WGS-84 | 32.3819°N, 64.6769°W |
| x_nm / y_nm | **+469.4 / +892.9** |
| Current ports.ron | (469.2, 892.8) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** St. George's Harbour is the principal harbour of Bermuda,
at the north-eastern end of the island chain. Wikipedia gives St. George's at
32°22′55″N 64°40′37″W = 32.3819°N, 64.6769°W. Bermuda was a crucial waypoint
for Atlantic crossings and famous for cedar-built sloops. Current value is exact.

**Sources:** Wikipedia "St. George's, Bermuda" (32°22′55″N 64°40′37″W).

---

#### Belize (*Belize Town / Belize River settlement*)
| Field | Value |
|---|---|
| WGS-84 | 17.4986°N, 88.1886°W |
| x_nm / y_nm | **−941.3 / −0.1** |
| Current ports.ron | (−942.0, 0.0) → Δ ~0.7 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** The Belize River settlement (forerunner of modern Belize City)
was established as a logwood-cutting camp from the 1630s–1650s at the mouth of
the Belize River on the Caribbean coast of what is now Belize. Wikipedia gives
Belize City at 17°29′55″N 88°11′19″W = 17.4986°N, 88.1886°W. This is exactly
at the Belize River mouth — the correct position for the c. 1680 settlement.
The current ports.ron value (−942.0, 0.0) = 17.500°N, 88.200°W is within 0.7 NM
of the Wikipedia coordinate and is essentially correct. No significant change
needed. The harbor_radius_nm: 30.0 is appropriate for this lagoon/barrier-reef
coastline where ships anchored offshore and lightered cargo in.

**Sources:** Wikipedia "Belize City" (17°29′55″N 88°11′19″W; settled 1638).

---

### 2.3 French Ports

---

#### Fort-Royal, Martinique
| Field | Value |
|---|---|
| WGS-84 | 14.6000°N, 61.0667°W |
| x_nm / y_nm | **+686.0 / −174.0** |
| Current ports.ron | (685.8, −174.0) → Δ ~0.2 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Fort-Royal (now Fort-de-France) is on the west coast of
Martinique in the Baie de Fort-de-France, a large natural harbour. Wikipedia
gives Fort-de-France at 14°36′N 61°04′W = 14.60°N, 61.07°W. Fort-Royal was the
principal French naval and commercial port in the Caribbean from 1680. Current
value is exact.

**Sources:** Wikipedia "Fort-de-France" (14°36′N 61°04′W).

---

#### Basse-Terre, Guadeloupe
| Field | Value |
|---|---|
| WGS-84 | 15.9833°N, 61.7333°W |
| x_nm / y_nm | **+646.0 / −91.0** |
| Current ports.ron | (646.2, −90.0) → Δ ~1 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Basse-Terre is on the southwest coast of Basse-Terre island
(western half of Guadeloupe) at the foot of the Soufrière volcano. The anchorage
is the roadstead off the town. Wikipedia gives 15°59′N 61°44′W = 15.9833°N,
61.733°W. Current value is essentially exact.

**Sources:** Wikipedia "Basse-Terre, Guadeloupe" (15°59′N 61°44′W).

---

#### Cap-Français (*Cap-Haïtien*)
| Field | Value |
|---|---|
| WGS-84 | 19.7600°N, 72.2000°W |
| x_nm / y_nm | **+18.0 / +135.6** |
| Current ports.ron | (18.0, 135.6) → Δ 0 NM |
| Status | ✅ **Exact match** |

**Rationale:** Cap-Français (now Cap-Haïtien) was the leading French colonial
port in Saint-Domingue by the 1680s, surpassing Tortuga as the Buccaneers'
main base moved south. The anchorage is the open roadstead north of the town
at 19.76°N, 72.20°W. Wikipedia exact. Current value is perfect.

**Sources:** Wikipedia "Cap-Haïtien" (19°45′36″N 72°12′00″W).

---

#### Petit-Goâve
| Field | Value |
|---|---|
| WGS-84 | 18.4314°N, 72.8669°W |
| x_nm / y_nm | **−22.0 / +55.9** |
| Current ports.ron | (−22.2, 55.8) → Δ ~0.3 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Petit-Goâve, on the southern peninsula of Hispaniola in the
Baie de Petit-Goâve, was the principal French buccaneer base in Saint-Domingue
from the 1670s and the seat of the French governor of Saint-Domingue. In 1678–83
it was the home port of many *flibustiers*. Wikipedia gives 18°26′N 72°52′W =
18.433°N, 72.867°W. Current value is essentially exact.

**Sources:** Wikipedia "Petit-Goâve" (18°26′05″N 72°52′01″W).

---

#### Cayenne, French Guiana
| Field | Value |
|---|---|
| WGS-84 | 4.9333°N, 52.3167°W |
| x_nm / y_nm | **+1211.0 / −754.0** |
| Current ports.ron | (1210.2, −753.6) → Δ ~0.9 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Cayenne is on an island at the mouth of the Cayenne River.
The anchorage is the roadstead off the town. Wikipedia gives 4°56′N 52°19′W =
4.933°N, 52.317°W. Current value is essentially exact. The harbor_radius_nm: 15.0
reflects the river-mouth approaches.

**Sources:** Wikipedia "Cayenne" (4°56′N 52°19′W).

---

### 2.4 Dutch Ports

---

#### Willemstad, Curaçao
| Field | Value |
|---|---|
| WGS-84 | 12.1170°N, 68.9330°W |
| x_nm / y_nm | **+214.0 / −323.0** |
| Current ports.ron | (214.2, −323.4) → Δ ~0.4 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Willemstad is on the south coast of Curaçao in the *Schottegat*
harbour, accessed through the narrow Sint Annabaai. The anchorage was the
outer roadstead in the Sint Annabaai approaches and the inner *Waaigat*.
Wikipedia gives 12°06′N 68°56′W = 12.10°N, 68.933°W. Current value is exact.
Willemstad was the great Dutch entrepôt for Spanish Main trade and slave
transshipment.

**Sources:** Wikipedia "Willemstad" (12°06′N 68°56′W, verified via geohack).

---

#### Oranjestad, Sint Eustatius
| Field | Value |
|---|---|
| WGS-84 | 17.4830°N, 62.9830°W |
| x_nm / y_nm | **+571.0 / −1.0** |
| Current ports.ron | (571.8, −1.2) → Δ ~0.8 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Sint Eustatius (*Statia*) was the leading Dutch free-port in
the Caribbean from the 1630s, reached its peak importance as a neutral entrepôt
in the 1770s–80s but was already a significant trading post in 1680.
The anchorage is the western roadstead (*Orange Bay*) off Oranjestad, open to
the sea as there is no sheltered harbour — hence ship turnover was high.
Wikipedia gives 17°29′N 62°59′W = 17.483°N, 62.983°W. Current value is exact.

**Sources:** Wikipedia "Oranjestad, Sint Eustatius" (17°29′N 62°59′W).

---

#### Paramaribo, Suriname
| Field | Value |
|---|---|
| WGS-84 | 5.9833°N, 55.1333°W |
| x_nm / y_nm | **+1042.0 / −691.0** |
| Current ports.ron | (1039.8, −685.0) → Δ **~6 NM south** |
| Status | ⚠️ Minor correction |

**Rationale:** Paramaribo is 15 km up the Suriname River from its mouth.
Ocean ships anchored at the river mouth roadstead at approximately 5.983°N,
55.133°W, not at the town (which would be further upstream). Wikipedia gives
Paramaribo city at 5°50′N 55°10′W = 5.833°N, 55.167°W. The current ports.ron
position (1039.8, −685.0) = ~5.983°N, 55.167°W corresponds to the river mouth
approach, which is the correct outer anchorage. However, a minor correction
(−6 NM south) moves the point to a more precise river mouth position.

**Sources:** Wikipedia "Paramaribo" (5°51′N 55°10′W city); river mouth
verified via approximate coordinates.

---

### 2.5 Pirate Havens

---

#### Tortuga (*Île de la Tortue*)
| Field | Value |
|---|---|
| WGS-84 | 20.0397°N, 72.7900°W |
| x_nm / y_nm | **−17.4 / +152.4** |
| Current ports.ron | (−16.8, 152.4) → Δ ~0.6 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Tortuga (Île de la Tortue), off the northwest coast of
Hispaniola, was the original buccaneer base from the 1630s–1660s before
gradually losing primacy to Petit-Goâve and Port Royal. The main anchorage
was the sheltered roadstead (*La Basse Terre*) on the south coast of the island
at approximately 20.04°N, 72.79°W. Wikipedia gives Tortuga at 20°02′N 72°47′W.
Current value is essentially exact.

**Sources:** Wikipedia "Tortuga (island)" (20°02′23″N 72°47′24″W).

---

#### Nassau, New Providence
| Field | Value |
|---|---|
| WGS-84 | 25.0781°N, 77.3386°W |
| x_nm / y_nm | **−290.3 / +454.7** |
| Current ports.ron | (−291.0, 453.0) → Δ ~2 NM |
| Status | ⚠️ Minor correction |

**Rationale:** Nassau (then *Charles Town* / *Nassau*) on New Providence Island
was the base of the Bahamian pirates during the 1680s–1710s. The harbour is
behind a bar on the north coast of New Providence at approximately 25.08°N,
77.34°W. Wikipedia gives Nassau at 25°04′41″N 77°20′19″W = 25.0781°N,
77.3386°W. Proposed position corrects by ~2 NM to the exact harbour location.

**Sources:** Wikipedia "Nassau, Bahamas" (25°04′41″N 77°20′19″W).

---

#### Tobago ⚠️ CONTESTED OWNERSHIP
| Field | Value |
|---|---|
| WGS-84 | 11.1833°N, 60.7375°W |
| x_nm / y_nm | **+705.8 / −379.0** |
| Current ports.ron | (709.8, −375.0) → Δ **~5 NM** |
| Status | ⚠️ Minor correction + **see Open Questions §4.4** |

**Rationale:** Tobago's main anchorage was *Scarborough Roads* off Scarborough
(*Lampsinsburg*) on the southwest coast of the island. Wikipedia gives
Scarborough at 11°11′N 60°44′W = 11.183°N, 60.733°W. The current ports.ron
position (709.8, −375.0) = ~11.25°N, 60.672°W is ~5 NM NNE of Scarborough
Roads, placing it incorrectly northeast of the harbour. The corrected position
is the actual Scarborough anchorage. Tobago's faction assignment as `Free` is
debatable — see Open Questions.

**Sources:** Wikipedia "Scarborough, Tobago" (11°11′N 60°44′W).

---

### 2.6 European Ports

---

#### London (*The Nore, Thames Estuary*)
| Field | Value |
|---|---|
| WGS-84 | 51.4833°N, 0.8833°E |
| x_nm / y_nm | **+4403.0 / +2039.0** |
| Current ports.ron | (4410.0, 2040.0) → Δ **~7 NM west** |
| Status | ⚠️ Minor correction |

**Rationale:** The Nore is the traditional anchorage at the mouth of the
Thames Estuary where ocean-going vessels waited for tidal windows and onward
lighter-passage to London Docks. Historically, the Nore sandbank lies at
approximately 51.48°N, 0.88°E; this was the Royal Navy's main exercise ground
and the anchorage for ships awaiting Thames pilots. The existing ports.ron
comment already identifies the Nore as the intended position, but the coordinate
(4410.0, 2040.0) = 51.5°N, 1.0°E places it further east. Corrected to 51.4833°N,
0.8833°E — the traditional Nore buoy position.

**Sources:** Wikipedia "The Nore" (51°29′N 0°53′E = 51.483°N, 0.883°E).

---

#### Amsterdam ⚠️ NAVIGATION RISK
| Field | Value |
|---|---|
| WGS-84 | 52.9833°N, 4.7833°E (Texel Roads) |
| x_nm / y_nm | **+4637.0 / +2129.0** |
| Current ports.ron | (4625.0, 2105.0) → Δ **~27 NM north** |
| Status | 🚨 **Significant change — see Open Questions §4.2** |

**Rationale:** The development log records that Amsterdam was already moved
from its original position to IJmuiden/North Sea (52.46°N, 4.59°E) to fix ~170
loitering ships caused by the harbour zone overlapping the Dutch coastline. The
historically correct outer anchorage for Amsterdam was **Texel Roads** (*Rede
van Texel*), the sheltered water between the island of Texel and the North
Holland coast at approximately 52.98°N, 4.78°E. This is where Dutch East India
(VOC) and West India (WIC) ships assembled before Atlantic departures and where
returning homeward-bound fleets anchored to offload into lighters for the
Zuyderzee passage. The proposed position is 27 NM north of the current
IJmuiden position. **HOWEVER:** moving this far north may re-introduce navmesh
connectivity problems (the 0.1 NM shore buffer may fail to create valid tiles
between Texel Roads and the open North Sea). Recommend testing before applying.
See §4.2 for fallback.

**Sources:** Wikipedia "Texel" (53°03′N 4°47′E for island); anchorage at
52.983°N, 4.783°E based on Texel Roads location south of island.

---

#### Cádiz, Spain
| Field | Value |
|---|---|
| WGS-84 | 36.5167°N, 6.2833°W |
| x_nm / y_nm | **+3973.0 / +1141.0** |
| Current ports.ron | (3972.0, 1142.0) → Δ ~1.3 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Cádiz is on a peninsula projecting into the Bay of Cádiz (Bahía
de Cádiz). The anchorage was in the outer bay (*Bahía exterior*) or in the
inner bay's deep channel near the city at approximately 36.517°N, 6.283°W.
Wikipedia gives Cádiz at 36°31′N 6°17′W = 36.517°N, 6.283°W. Cádiz was the
official port of departure for all Spanish fleet sailings to the Americas after
1680 (replacing Seville). The Royal shipyard (Carraca) was at nearby La Isla de
San Fernando.

**Sources:** Wikipedia "Cádiz" (36°31′N 6°17′W).

---

#### Nantes (*Paimboeuf / Loire Estuary*)
| Field | Value |
|---|---|
| WGS-84 | 47.2833°N, 2.2000°W |
| x_nm / y_nm | **+4218.0 / +1787.0** |
| Current ports.ron | (4218.0, 1788.0) → Δ ~1 NM |
| Status | ✅ Essentially unchanged |

**Rationale:** Nantes proper is 50 km up the Loire from the sea; ocean-going
ships used the anchorage at **Paimboeuf** or Saint-Nazaire at the Loire estuary
mouth. The ports.ron comment already correctly identifies this. The current
coordinate (4218.0, 1788.0) = 47.30°N, 2.20°W = approximately Saint-Nazaire /
outer Loire estuary mouth. Wikipedia gives Saint-Nazaire at 47°16′N 2°12′W =
47.267°N, 2.200°W and Paimboeuf at ~47.283°N, 2.034°W. The current value is
a reasonable midpoint and a de minimis change is needed; keep as-is.

**Sources:** Wikipedia "Saint-Nazaire" (47°16′N 2°12′W);
Wikipedia "Paimboeuf" (47°17′N 2°02′W).

---

### 2.7 West African Ports

---

#### Elmina (*São Jorge da Mina*)
| Field | Value |
|---|---|
| WGS-84 | 5.0833°N, 1.3500°W |
| x_nm / y_nm | **+4269.0 / −745.0** |
| Current ports.ron | (4269.0, −745.0) → Δ 0 NM |
| Status | ✅ **Exact match** |

**Rationale:** Elmina Castle (*Castelo de São Jorge da Mina*), on Ghana's
Cape Coast, was the oldest European fort in sub-Saharan Africa (1482 Portuguese,
Dutch from 1637). The anchorage was the open roadstead directly in front of
the castle at approximately 5.083°N, 1.350°W. Wikipedia gives Elmina at
5°05′N 1°21′W = 5.083°N, 1.350°W. Current value is exact. Dutch WIC held Elmina
as primary Gold Coast factor until British seizure in 1872.

**Sources:** Wikipedia "Elmina" (5°05′N 1°21′W).

---

#### Ouidah (*Whydah / Juda*)
| Field | Value |
|---|---|
| WGS-84 | 6.3000°N, 2.0833°E |
| x_nm / y_nm | **+4475.0 / −672.0** |
| Current ports.ron | (4470.0, −666.0) → Δ **~6 NM south/east** |
| Status | ⚠️ Minor correction |

**Rationale:** Ouidah (Whydah, Juda) on the Bight of Benin was the principal
slave-trade port of the Dahomey coast from the 1640s. Unlike Elmina, which has a
near-shore anchorage, Ouidah is on a low surf-beaten coast with no natural
harbour; European ships anchored **2–3 nautical miles offshore** in the open
roadstead while slaves were canoed out through heavy surf. The Wikipedia town
coordinate is 6.367°N, 2.083°E (on the shore), but the historical anchorage
was approximately 6.30°N, 2.08°E — 4 km/2 NM south in the open sea.
The current ports.ron position (4470.0, −666.0) = 6.40°N, 2.00°E is actually
north of the town (offshore would be south). Proposed value (4475.0, −672.0) =
6.30°N, 2.083°E corrects to the offshore anchorage ~2 NM south of shore. The
harbor_radius_nm: 15.0 is appropriate for this open roadstead.

**Sources:** Wikipedia "Ouidah" (6°21′N 2°05′E town); anchorage offset from
NGA Pub. 140 West Africa sailing directions (historical offshore roads convention
for this coast).

---

## 3. Complete Proposed `ports.ron` Positions

The following table lists all 38 ports with their current and proposed positions.
Entries marked ✅ are unchanged or differ by <2 NM; ⚠️ differ 2–10 NM; 🚨 differ >10 NM.

| Port | Current (x, y) | Proposed (x, y) | Δ NM | Status |
|---|---|---|---|---|
| Havana | (−592.8, 337.8) | (−591.6, 338.7) | ~1.5 | ✅ |
| Portobelo | (−429.0, −477.0) | (−429.3, −476.7) | ~0.4 | ✅ |
| Cartagena | (−181.8, −426.0) | (−184.0, −435.0) | ~9 | ⚠️ |
| Santo Domingo | (157.2, 58.2) | (156.0, 58.0) | ~1.2 | ✅ |
| Santiago de Cuba | (−199.2, 151.2) | (−203.0, 149.0) | ~4 | ⚠️ |
| San Juan | (382.8, 58.2) | (383.0, 58.0) | ~0.2 | ✅ |
| Maracaibo | (52.2, −412.2) | (54.0, −391.0) | **~21** | 🚨 |
| La Guaira | (334.2, −414.0) | (334.0, −414.0) | ~0.2 | ✅ |
| Trinidad | (658.8, −411.0) | (659.0, −411.0) | ~0.2 | ✅ |
| Margarita | (517.8, −390.0) | (522.3, −390.0) | ~4.5 | ⚠️ |
| Port Royal | (−260.4, 26.4) | (−260.5, 26.3) | ~0.2 | ✅ |
| Kingston | (−257.4, 28.2) | (−260.5, 26.3) | ~4 | 🚨 see §4.1 |
| Bridgetown | (772.8, −264.0) | (772.0, −264.0) | ~0.8 | ✅ |
| Basseterre | (586.8, −12.0) | (587.0, −12.0) | ~0.2 | ✅ |
| English Harbour | (644.4, −30.0) | (644.0, −30.0) | ~0.4 | ✅ |
| Charleston | (−445.8, 916.8) | (−443.0, 914.0) | ~4 | ⚠️ |
| Boston | (86.4, 1491.6) | (93.0, 1489.0) | ~7 | ⚠️ |
| New York | (−90.6, 1392.6) | (−91.0, 1391.0) | ~1.7 | ✅ |
| Philadelphia | (−177.8, 1312.6) | (−156.0, 1284.0) | **~38** | 🚨 |
| Bermuda | (469.2, 892.8) | (469.4, 892.9) | ~0.2 | ✅ |
| Belize | (−942.0, 0.0) | (−941.3, −0.1) | ~0.7 | ✅ |
| Fort-Royal | (685.8, −174.0) | (686.0, −174.0) | ~0.2 | ✅ |
| Basse-Terre | (646.2, −90.0) | (646.0, −91.0) | ~1 | ✅ |
| Cap-Français | (18.0, 135.6) | (18.0, 135.6) | 0 | ✅ |
| Petit-Goâve | (−22.2, 55.8) | (−22.0, 55.9) | ~0.3 | ✅ |
| Cayenne | (1210.2, −753.6) | (1211.0, −754.0) | ~0.9 | ✅ |
| Willemstad | (214.2, −323.4) | (214.0, −323.0) | ~0.4 | ✅ |
| St. Eustatius | (571.8, −1.2) | (571.0, −1.0) | ~0.8 | ✅ |
| Paramaribo | (1039.8, −685.0) | (1042.0, −691.0) | ~6 | ⚠️ |
| Tortuga | (−16.8, 152.4) | (−17.4, 152.4) | ~0.6 | ✅ |
| Nassau | (−291.0, 453.0) | (−290.3, 454.7) | ~2 | ⚠️ |
| Tobago | (709.8, −375.0) | (705.8, −379.0) | ~5 | ⚠️ |
| London | (4410.0, 2040.0) | (4403.0, 2039.0) | ~7 | ⚠️ |
| Amsterdam | (4625.0, 2105.0) | (4637.0, 2129.0) | **~27** | 🚨 see §4.2 |
| Cadiz | (3972.0, 1142.0) | (3973.0, 1141.0) | ~1.3 | ✅ |
| Nantes | (4218.0, 1788.0) | (4218.0, 1788.0) | 0 | ✅ |
| Elmina | (4269.0, −745.0) | (4269.0, −745.0) | 0 | ✅ |
| Ouidah | (4470.0, −666.0) | (4475.0, −672.0) | ~6 | ⚠️ |

---

## 4. Open Questions

### 4.1 Kingston — Anachronism

**Problem:** Kingston was founded in 1692–93, twelve years after the sim's
1680 scenario begins. It does not exist. The current ports.ron has both Port
Royal and Kingston as separate entries with different positions.

**Options:**
- **A (recommended):** Remove the Kingston entry. Port Royal covers the same
  harbour and is the historically correct 1680 entry. Any game references to
  "Kingston" by name would need updating.
- **B:** Retain Kingston but set its `position` to Port Royal's anchorage
  (−260.5, 26.3) so both entries refer to the same harbour, treating Kingston
  as a ghost/placeholder for post-earthquake continuity if the scenario
  is designed to extend beyond 1692.
- **C:** Keep Kingston at a slightly different position inside Kingston Harbour
  to represent the pre-earthquake settlement that eventually became Kingston;
  but this is historically dubious (no town existed there in 1680).

### 4.2 Amsterdam — Texel Roads vs. IJmuiden (Navigation Risk)

**Problem:** The development log (lines 2156–2204) documents that Amsterdam was
already moved from its original position to IJmuiden (52.46°N, 4.59°E) to fix
~170 loitering ships caused by the harbour zone overlapping the Dutch coastline.
The proposed Texel Roads position (52.98°N, 4.78°E) is 27 NM north of IJmuiden.
Moving this far north may re-introduce navmesh connectivity problems if the 0.1 NM
shore buffer creates isolated tiles between the Texel Roads position and the
open North Sea approach.

**Recommendation:** Before applying the Texel Roads correction, verify in the
navmesh preprocessing output that:
1. Tiles exist at 52.98°N, 4.78°E
2. Those tiles connect (same navmesh component) to the open Atlantic tiles
3. The harbor_radius_nm: 25.0 zone doesn't clip the Texel island coastline

**Fallback:** If Texel Roads causes navmesh failures, keep the IJmuiden
position (4625.0, 2105.0) and annotate it as "IJmuiden outer roads (historically
approximate for Texel Roads)."

### 4.3 Philadelphia — Chronological Ambiguity

**Problem:** Philadelphia was not founded until 1682 (Penn's Charter 1681,
city laid out 1682). Strictly speaking it does not exist in 1680.

**Options:**
- **A:** Remove Philadelphia from the 1680 starting configuration; add it
  as a deferred-spawn port that becomes active in 1682 or later.
- **B (current approach):** Retain it as representing the future colony's
  region, corrected to the outer Delaware Bay anchorage (−156.0, 1284.0).
  The Philadelphia shipyard (`Some(["sloop", "brigantine"])`) makes more sense
  from 1683+ when the colony was established.

### 4.4 Tobago — Contested Ownership

**Problem:** Tobago changed hands repeatedly in the 17th century (Courland/Latvia
1654–1659, Dutch 1659–1665, English 1672–1677, French 1677–1681, then back
to England). In 1680, Tobago was under nominal French administration following
the 1677 French seizure, though contested. The current `faction: Free` /
`category: PirateHaven` assignment is historically debatable; by 1680 it was a
de facto French possession, not a pirate haven.

**Recommendation:** Change to `faction: France` and `category: SmallColonial`
for strict 1680 accuracy. Or retain `Free` if the designer intends Tobago as
an uncontrolled contested zone.

### 4.5 Maracaibo — harbor_radius_nm

The corrected Maracaibo anchorage at the San Carlos bar (54.0, −391.0) is
~21 NM north of the current position, which was inside the lake near the city.
The current `harbor_radius_nm: 30.0` was presumably intended to reach from the
outer water into the lake settlement. After moving to the bar, the radius should
still be 30.0 (or larger — test for navmesh tile coverage) to ensure the harbour
zone encompasses the shallow-water transit up the strait to the city.

---

## 5. Copy-Pasteable `ports.ron` Diff

The following lists **only the `position:` field** for all 38 ports; all other
fields are unchanged from the current `data/registries/ports.ron`. Positions
that are essentially unchanged (<2 NM) are still shown for completeness.

```ron
// === SPANISH ===
(name: "Havana",          position: (-591.6, 338.7),  ...),
(name: "Portobelo",       position: (-429.3, -476.7), ...),
(name: "Cartagena",       position: (-184.0, -435.0), ...),  // ~9 NM correction
(name: "Santo Domingo",   position: (156.0, 58.0),    ...),
(name: "Santiago de Cuba",position: (-203.0, 149.0),  ...),  // ~4 NM correction
(name: "San Juan",        position: (383.0, 58.0),    ...),
(name: "Maracaibo",       position: (54.0, -391.0),   ...),  // ~21 NM — bar/narrows
(name: "La Guaira",       position: (334.0, -414.0),  ...),
(name: "Trinidad",        position: (659.0, -411.0),  ...),
(name: "Margarita",       position: (522.3, -390.0),  ...),  // ~4.5 NM — Pampatar

// === ENGLISH ===
(name: "Port Royal",      position: (-260.5, 26.3),   ...),
(name: "Kingston",        position: (-260.5, 26.3),   ...),  // ⚠️ see §4.1 — anachronism
(name: "Bridgetown",      position: (772.0, -264.0),  ...),
(name: "Basseterre",      position: (587.0, -12.0),   ...),
(name: "English Harbour", position: (644.0, -30.0),   ...),
(name: "Charleston",      position: (-443.0, 914.0),  ...),  // ~4 NM — outer bar
(name: "Boston",          position: (93.0, 1489.0),   ...),  // ~7 NM — Nantasket Roads
(name: "New York",        position: (-91.0, 1391.0),  ...),
(name: "Philadelphia",    position: (-156.0, 1284.0), ...),  // ~38 NM — Cape Henlopen
(name: "Bermuda",         position: (469.4, 892.9),   ...),
(name: "Belize",          position: (-941.3, -0.1),   ...),

// === FRENCH ===
(name: "Fort-Royal",      position: (686.0, -174.0),  ...),
(name: "Basse-Terre",     position: (646.0, -91.0),   ...),
(name: "Cap-Français",    position: (18.0, 135.6),    ...),  // exact match
(name: "Petit-Goâve",     position: (-22.0, 55.9),    ...),
(name: "Cayenne",         position: (1211.0, -754.0), ...),

// === DUTCH ===
(name: "Willemstad",      position: (214.0, -323.0),  ...),
(name: "St. Eustatius",   position: (571.0, -1.0),    ...),
(name: "Paramaribo",      position: (1042.0, -691.0), ...),  // ~6 NM — river mouth

// === PIRATE HAVENS ===
(name: "Tortuga",         position: (-17.4, 152.4),   ...),
(name: "Nassau",          position: (-290.3, 454.7),  ...),  // ~2 NM correction
(name: "Tobago",          position: (705.8, -379.0),  ...),  // ~5 NM — Scarborough Roads

// === EUROPE ===
(name: "London",          position: (4403.0, 2039.0), ...),  // ~7 NM — precise Nore
(name: "Amsterdam",       position: (4637.0, 2129.0), ...),  // ~27 NM — Texel Roads ⚠️ test nav
(name: "Cadiz",           position: (3973.0, 1141.0), ...),
(name: "Nantes",          position: (4218.0, 1788.0), ...),  // unchanged

// === WEST AFRICA ===
(name: "Elmina",          position: (4269.0, -745.0), ...),  // exact match
(name: "Ouidah",          position: (4475.0, -672.0), ...),  // ~6 NM S — offshore roads
```

### Complete replacement lines (full RON, drop-in for `ports.ron`):

```ron
[
    // === SPANISH ===
    (name: "Havana",          position: (-591.6, 338.7),  faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: CaribbeanEntrepot),
    (name: "Portobelo",       position: (-429.3, -476.7), faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Cartagena",       position: (-184.0, -435.0), faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: CaribbeanEntrepot),
    (name: "Santo Domingo",   position: (156.0, 58.0),    faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Santiago de Cuba",position: (-203.0, 149.0),  faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "San Juan",        position: (383.0, 58.0),    faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    // Maracaibo: anchorage at San Carlos bar/narrows entrance to Lake Maracaibo (~11°N).
    // Wide radius to cover transit to city inside the lake.
    (name: "Maracaibo",       position: (54.0, -391.0),   faction: Spain,       harbor_radius_nm: 30.0, shipyard: None, category: SmallColonial),
    (name: "La Guaira",       position: (334.0, -414.0),  faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Trinidad",        position: (659.0, -411.0),  faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    // Margarita: Pampatar anchorage (NE coast), not mid-island.
    (name: "Margarita",       position: (522.3, -390.0),  faction: Spain,       harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),

    // === ENGLISH ===
    (name: "Port Royal",      position: (-260.5, 26.3),   faction: England,     harbor_radius_nm: 8.0,  shipyard: None, category: CaribbeanEntrepot),
    // Kingston: anachronism — founded 1692 after Port Royal earthquake. Placed at Port Royal
    // anchorage as placeholder. Consider removing for strict 1680 scenario (see research/port-coordinates.md §4.1).
    (name: "Kingston",        position: (-260.5, 26.3),   faction: England,     harbor_radius_nm: 8.0,  shipyard: None, category: CaribbeanEntrepot),
    (name: "Bridgetown",      position: (772.0, -264.0),  faction: England,     harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Basseterre",      position: (587.0, -12.0),   faction: England,     harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "English Harbour", position: (644.0, -30.0),   faction: England,     harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    // Charleston: outer bar anchorage (~4 NM SE of city centre).
    (name: "Charleston",      position: (-443.0, 914.0),  faction: England,     harbor_radius_nm: 25.0, shipyard: None, category: SmallColonial),
    // Boston: Nantasket/King's Roads outer roadstead.
    (name: "Boston",          position: (93.0, 1489.0),   faction: England,     harbor_radius_nm: 20.0, shipyard: Some(["sloop", "brigantine", "bark"]), category: SmallColonial),
    (name: "New York",        position: (-91.0, 1391.0),  faction: England,     harbor_radius_nm: 60.0, shipyard: None, category: SmallColonial),
    // Philadelphia: outer Delaware Bay approaches off Cape Henlopen (~38.9°N).
    // Philadelphia barely exists in 1680 (founded 1682); consider deferred spawn.
    (name: "Philadelphia",    position: (-156.0, 1284.0), faction: England,     harbor_radius_nm: 30.0, shipyard: Some(["sloop", "brigantine"]), category: SmallColonial),
    (name: "Bermuda",         position: (469.4, 892.9),   faction: England,     harbor_radius_nm: 8.0,  shipyard: Some(["sloop"]), category: SmallColonial),
    // Belize: Belize River mouth settlement (logwood cutters); essentially unchanged.
    (name: "Belize",          position: (-941.3, -0.1),   faction: England,     harbor_radius_nm: 30.0, shipyard: None, category: SmallColonial),

    // === FRENCH ===
    (name: "Fort-Royal",      position: (686.0, -174.0),  faction: France,      harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Basse-Terre",     position: (646.0, -91.0),   faction: France,      harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Cap-Français",    position: (18.0, 135.6),    faction: France,      harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    (name: "Petit-Goâve",     position: (-22.0, 55.9),    faction: France,      harbor_radius_nm: 8.0,  shipyard: None, category: PirateHaven),
    (name: "Cayenne",         position: (1211.0, -754.0), faction: France,      harbor_radius_nm: 15.0, shipyard: None, category: SmallColonial),

    // === DUTCH ===
    (name: "Willemstad",      position: (214.0, -323.0),  faction: Netherlands, harbor_radius_nm: 8.0,  shipyard: None, category: CaribbeanEntrepot),
    (name: "St. Eustatius",   position: (571.0, -1.0),    faction: Netherlands, harbor_radius_nm: 8.0,  shipyard: None, category: SmallColonial),
    // Paramaribo: river-mouth outer anchorage (~6 NM S of city).
    (name: "Paramaribo",      position: (1042.0, -691.0), faction: Netherlands, harbor_radius_nm: 15.0, shipyard: None, category: SmallColonial),

    // === PIRATE HAVENS ===
    (name: "Tortuga",         position: (-17.4, 152.4),   faction: Free,        harbor_radius_nm: 8.0,  shipyard: None, category: PirateHaven),
    (name: "Nassau",          position: (-290.3, 454.7),  faction: Free,        harbor_radius_nm: 8.0,  shipyard: None, category: PirateHaven),
    // Tobago: Scarborough Roads anchorage. Contested ownership in 1680 (nominally French);
    // kept as Free per original intent — see research/port-coordinates.md §4.4.
    (name: "Tobago",          position: (705.8, -379.0),  faction: Free,        harbor_radius_nm: 8.0,  shipyard: None, category: PirateHaven),

    // === EUROPE ===
    // London at The Nore (51.48°N 0.88°E), Thames Estuary outer anchorage.
    (name: "London",          position: (4403.0, 2039.0), faction: England,     harbor_radius_nm: 30.0, shipyard: Some(["brigantine", "bark", "ship"]), category: EuropeanHub),
    // Amsterdam at Texel Roads (52.98°N 4.78°E) — historically correct outer anchorage.
    // ⚠️ TEST navmesh connectivity before applying: prior IJmuiden fix (4625.0, 2105.0)
    // resolved loitering issues; verify Texel Roads tiles exist and connect to open North Sea.
    // Fallback if navmesh fails: revert to (4625.0, 2105.0) IJmuiden position.
    (name: "Amsterdam",       position: (4637.0, 2129.0), faction: Netherlands, harbor_radius_nm: 25.0, shipyard: Some(["fluyt", "ship"]), category: EuropeanHub),
    (name: "Cadiz",           position: (3973.0, 1141.0), faction: Spain,       harbor_radius_nm: 20.0, shipyard: Some(["ship"]), category: EuropeanHub),
    // Nantes at Loire estuary mouth / Saint-Nazaire area (47.3°N 2.2°W) — unchanged.
    (name: "Nantes",          position: (4218.0, 1788.0), faction: France,      harbor_radius_nm: 25.0, shipyard: Some(["bark", "ship"]), category: EuropeanHub),

    // === WEST AFRICA ===
    (name: "Elmina",          position: (4269.0, -745.0), faction: Netherlands, harbor_radius_nm: 15.0, shipyard: None, category: CaribbeanEntrepot),
    // Ouidah: 2 NM offshore roadstead (ships anchored off surf-beaten coast, not on shore).
    (name: "Ouidah",          position: (4475.0, -672.0), faction: France,      harbor_radius_nm: 15.0, shipyard: None, category: CaribbeanEntrepot),
]
```

---

*Document generated from Wikipedia coordinate verification and historical*
*anchorage research. All coordinates WGS-84. Verified 2025.*
