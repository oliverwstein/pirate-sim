# Caribbean Geopolitical-Economic Simulation: Research Manifest
## Period: 1600–1720 | Scope: Caribbean theater, Americas, with Europe/Africa as exogenous boundary

### What This Document Is

This is a research specification for a historical geopolitical-economic simulation set in the Caribbean and broader Atlantic world, roughly 1600–1720. The simulation sits somewhere between Sid Meier's Pirates! and Victoria II: it models trade, production, colonial development, naval and land warfare, diplomacy, and the interaction between metropolitan European powers and their Caribbean colonies. The player(s) may control one or more factions; the simulation should also run autonomously with AI agents.

Europe's internal politics are largely exogenous — wars, policy changes, and dynastic events happen on a schedule or stochastically, and the player cannot significantly alter them. However, the Caribbean consequences of those events (who is at war with whom, what trade restrictions are in effect, what military resources are deployed) are modeled in full. The Americas (North and South) are modeled with intermediate fidelity — they affect the Caribbean through trade and geopolitics but are not the primary theater.

Each section below describes a **simulation system**, what it needs to do, and what historical data is required to parameterize it. The research goal is not to write a history of the Caribbean but to extract the numbers, ratios, and structural facts needed to make each system work.

---

## 1. The Map: Geography and Topology

### What the simulation needs
A spatial model of the Caribbean and surrounding coasts, including: islands, ports, sea zones, prevailing winds and currents, distances between ports, and hazard zones (reefs, hurricane paths). The map must support pathfinding for ships with variable sailing characteristics (windward ability, speed in different wind conditions).

### Research questions
- What were the major ports and settlements in the Caribbean, 1600–1720? For each: location, founding date, controlling power, approximate population at benchmark dates (1600, 1650, 1700).
- What were the prevailing wind patterns and seasonal currents? How did they affect sailing times between major ports? What was the hurricane season and what were its effects on shipping?
- What were the major sea lanes and why? (i.e., which routes were favored by wind/current, which were avoided due to hazards)
- What natural harbors and anchorages existed, and what made a port site valuable? (depth, shelter, fresh water, defensibility)
- How long did typical voyages take between major port pairs under normal conditions? Under adverse conditions?

### Desired output
- A list of 40–80 settlements/ports with coordinates, founding dates, controlling power over time, and basic geographic characteristics.
- A wind/current model or reference sufficient to build one: prevailing directions by season, typical speeds, hurricane season boundaries and frequency.
- A distance/transit-time matrix for major port pairs, ideally with seasonal variation.
- A list of geographic hazards (reef zones, lee shores, passages) with locations.

### Starting points
- Admiralty sailing directions for the Caribbean (historical and modern — the winds haven't changed).
- Andrews, *The Spanish Caribbean: Trade and Plunder, 1530–1630* (1978).
- Jarvis, *In the Eye of All Trade: Bermuda, Bermudians, and the Maritime Atlantic World, 1680–1783* (2010).
- Historical atlases of the Caribbean, particularly those showing settlement dates and colonial boundaries.
- Mulcahy, *Hurricanes and Society in the British Greater Caribbean, 1624–1783* (2008) — for hurricane patterns and their impact.

---

## 2. Factions: The Powers and Their Interests

### What the simulation needs
A model of each faction's goals, resources, constraints, and decision-making. Factions include the metropolitan European powers (Spain, England, France, Holland) and potentially semi-autonomous colonial entities, pirates/buccaneers, and indigenous groups. Each faction has an economic base (partly exogenous for European powers), military assets, diplomatic relationships, and policy levers (tariffs, trade restrictions, military deployments, colonial charters).

### Research questions
- What were each European power's strategic objectives in the Caribbean, and how did they change over the period? (e.g., Spain: defend silver routes and territorial monopoly; England: break Spanish monopoly, develop plantation colonies; France: same as England but later; Holland: commercial dominance through free trade and entrepots)
- What resources did each power commit to the Caribbean? Naval forces (how many ships, of what types, stationed where), army garrisons, administrative apparatus, subsidies to colonies.
- How were colonial governors appointed and what authority did they have? What decisions were made locally vs. directed from the metropole? What was the communication lag?
- What was the structure of piracy and buccaneering? How were pirate groups organized, funded, and based? What was the relationship between piracy and privateering (letters of marque)? How did pirate activity change over the period?
- What was the role of indigenous peoples (Caribs, etc.) in the geopolitics of the period? Where were they significant military actors?
- What were the major alliances, rivalries, and wars between European powers during this period, and what were their Caribbean manifestations? (Eighty Years' War, Anglo-Dutch Wars, Nine Years' War, War of Spanish Succession, etc.)

### Desired output
- Faction profiles for each major power: objectives, available resources, policy instruments, key constraints.
- A timeline of European wars and their Caribbean theaters: dates, belligerents, major actions, territorial outcomes, and effects on trade.
- A model of the metropolitan decision-making process: what triggers fleet deployments, changes in trade policy, declaration of war/peace in the Caribbean specifically.
- Pirate/buccaneer faction profile: recruitment mechanics, base locations, organizational structure, relationship to legitimate powers.

### Starting points
- Haring, *The Spanish Empire in America* (1947).
- Dunn, *Sugar and Slaves: The Rise of the Planter Class in the English West Indies, 1624–1713* (1972).
- Pritchard, *In Search of Empire: The French in the Americas, 1670–1730* (2004).
- Klooster, *The Dutch Moment: War, Trade, and Settlement in the Seventeenth-Century Atlantic World* (2016).
- Exquemelin, *The Buccaneers of America* (1678/1684) — primary source on buccaneer organization.
- Rediker, *Villains of All Nations: Atlantic Pirates in the Golden Age* (2004).
- Lane, *Pillaging the Empire: Piracy in the Americas, 1500–1750* (1998).

---

## 3. The Economy: Goods, Production, and Trade

### What the simulation needs
An economic model in which settlements produce goods from local resources and imported inputs, consume goods to sustain their populations and grow, and trade surpluses for things they lack. The model must handle: production (what each settlement can make and at what cost), consumption (what populations and industries require), trade (moving goods between ports at a cost), and prices (emerging from supply and demand, modified by tariffs and restrictions).

### 3a. Goods Taxonomy

#### Research questions
- What were the major traded goods in the Caribbean, 1600–1720? For each: what was it, where was it produced, who consumed it, what was its approximate value per unit weight, and how bulky/perishable was it?
- What inputs did each good require for production? (e.g., sugar requires: suitable land, enslaved labor, cattle for mills, fuel/timber for boiling houses, copper kettles and other metalwork, food to feed workers)
- Which goods were substitutable? (e.g., could a colony switch from sugar to tobacco if prices shifted?)
- Which goods were essential for settlement survival vs. discretionary?
- What goods did Europe export to the Caribbean? What goods did Africa supply? What did North America supply?

#### Desired output
A goods table with:
- Name, category, typical unit (hogshead, ton, etc.)
- Value per unit at a reference port and date
- Value-to-weight ratio (determines what's worth shipping long distances)
- Perishability / storage requirements
- Production inputs required
- Where produced, where consumed
- Strategic classification: subsistence necessity, trade commodity, luxury, military supply, capital good

### 3b. Production Model

#### Research questions
- What was the production process for each major good? What determined output levels? (land area, labor quantity, capital investment, input availability)
- How long did it take to establish a new plantation or mine from scratch? What was the capital investment required?
- What were the yields per unit of land/labor for major crops in different conditions?
- How did soil exhaustion, deforestation, and other environmental factors affect production over time?
- What manufacturing existed in the colonies? (Sugar processing, rum distilling, shipbuilding, metalworking, textile production) What were its limits?

#### Desired output
- Production functions for major goods: inputs → output, with approximate ratios.
- Establishment timelines and costs for new production (e.g., a sugar plantation takes X years and Y capital to reach full production).
- Environmental degradation rates where relevant.

### 3c. Trade Mechanics

#### Research questions
- What were the transport costs for shipping goods between major ports? (per ton, by ship type, by route)
- How did merchants decide what to buy and sell where? What information did they have about prices at distant ports?
- What was the role of factors, agents, and correspondents in organizing trade?
- How was trade financed? (bills of exchange, trade credit, consignment, cash)
- What was the volume of smuggling relative to legal trade? How was it organized?
- How large were merchant fleets by nationality and how did they grow?

#### Desired output
- Transport cost model: cost per ton per nautical mile (or per voyage leg) by ship type.
- Information lag model: how quickly did price information travel between ports?
- Smuggling model: under what conditions does smuggling become rational, and at what scale?

### 3d. Prices and Markets

#### Research questions
- What were prices for major goods at major ports, at benchmark dates?
- How did prices respond to supply gluts or shortages? (Specifically: the tobacco price collapse as Virginia production scaled; sugar price fluctuations with new island development)
- Were there price controls or administered prices for any goods? (e.g., Spanish bullion, regulated commodity prices)
- How did wartime disruption affect prices?

#### Desired output
- Benchmark price tables for major goods at major ports (1620, 1650, 1680, 1700, 1720).
- Documented price elasticities or at least directional evidence of price responses to supply changes.
- Wartime price premium estimates.

### Starting points for all of section 3
- McCusker and Menard, *The Economy of British America, 1607–1789* (1985).
- Sheridan, *Sugar and Slavery: An Economic History of the British West Indies, 1623–1775* (1974).
- Menard, "A Note on Chesapeake Tobacco Prices, 1618–1660," *Virginia Magazine of History and Biography* 84 (1976).
- Mintz, *Sweetness and Power: The Place of Sugar in Modern History* (1985).
- Davis, *The Rise of the English Shipping Industry in the Seventeenth and Eighteenth Centuries* (1962).
- Klooster, *Illicit Riches: Dutch Trade in the Caribbean, 1648–1795* (2003).
- Zahedieh, *The Capital and the Colonies: London and the Atlantic Economy, 1660–1700* (2010).
- Topik, Marichal, and Frank (eds.), *From Silver to Cocaine* (2006).
- Cuenca Esteban, "Statistics of Spain's Colonial Trade, 1792–1820," *HAHR* 61:3 (1981).
- Cole, *Wholesale Commodity Prices in the United States, 1700–1861* (1938).
- McCusker, *Rum and the American Revolution* (1989).

---

## 4. Tariffs, Trade Restrictions, and Mercantile Policy

### What the simulation needs
A rule system governing who can trade what, where, and at what cost. This is central to the simulation because mercantile restrictions create the price differentials that drive smuggling, piracy, and inter-imperial competition. The restrictions also define the economic relationship between colonies and their metropoles.

### Research questions
- What were the specific trade restriction regimes for each power?
  - **Spain**: Casa de Contratación monopoly, convoy system (flotas and galeones), licensed ports, restrictions on inter-colonial trade, foreign exclusion. How did these change with the Bourbon reforms?
  - **England**: Navigation Acts (1651, 1660, 1663, 1673 iterations). Enumerated goods, colonial port restrictions, ship nationality requirements, Staple Act requirements. Plantation duty.
  - **France**: Exclusif system. Which ports could trade with colonies? What goods were restricted? How strictly enforced?
  - **Holland**: Relatively open trade policy. What restrictions existed? How did Dutch free ports (Curaçao, St. Eustatius) function as smuggling entrepots?
- What were the actual tariff rates (ad valorem or specific) on major goods under each regime?
- What were the penalties for smuggling, and how often were they enforced?
- What was the estimated volume of smuggling as a fraction of legal trade, by region and period?
- How did wartime affect trade restrictions? (trading with the enemy, neutral flags, prize law)
- What role did privateering commissions (letters of marque) play in wartime trade disruption?

### Desired output
- A per-faction tariff and restriction schedule: for each metropolitan power, what goods can be traded, at what ports, with what duties, under what conditions.
- Enforcement model: probability of detection for smuggling, by region and period, and consequences of detection.
- Wartime modifications to trade rules.
- Privateering economics: cost to outfit a privateer, expected returns, legal framework.

### Starting points
- Harper, *The English Navigation Laws: A Seventeenth-Century Experiment in Social Engineering* (1939).
- Fisher, *Commercial Relations between Spain and Spanish America in the Era of Free Trade, 1778–1796* (1985).
- Pares, *War and Trade in the West Indies, 1739–1763* (1936).
- Klooster, *Illicit Riches* (2003).
- Stein and Stein, *Silver, Trade, and War: Spain and America in the Making of Early Modern Europe* (2000).
- Starkey, *British Privateering Enterprise in the Eighteenth Century* (2015).
- Lydon, *Pirates, Privateers, and Profits* (1970).

---

## 5. Ships and Naval Warfare

### What the simulation needs
A model of the ships that carry trade and fight wars. Ships are the fundamental mobile unit of the simulation — they move goods, project military power, enforce trade restrictions, and conduct piracy. The model must cover: ship types and their characteristics, construction, crewing, operating costs, combat capabilities, and the tactical/strategic principles governing their use.

### 5a. Ship Types and Characteristics

#### Research questions
- What were the major ship types in Caribbean waters, 1600–1720? For each: tonnage range, crew requirements (minimum to sail, full complement), cargo capacity, armament (number and weight of guns), speed under various wind conditions, windward sailing ability, seaworthiness, typical cost to build, typical cost to operate per month.
- How did ship types evolve over the period? (e.g., the transition from galleons to frigates; the development of purpose-built slave ships; the emergence of fast sloops for smuggling/piracy)
- Where were ships built? What materials and skills were required? How long did construction take?
- What was the typical operational lifespan of a ship? How did maintenance (careening, repair) affect availability?
- What were crew wages by role (captain, mate, sailor, gunner, carpenter, surgeon)? How were pirate crews compensated differently from naval or merchant crews?

#### Desired output
- A ship type table covering roughly 8–15 types (pinnace, sloop, brigantine, bark/barque, merchantman, flute/fluyt, frigate, galleon, ship of the line, war canoe, etc.) with all characteristics listed above.
- Shipbuilding requirements: materials, labor, time, cost by type.
- Crew wage tables by role, nationality, and period.
- Operating cost model: crew wages + victualling + maintenance + insurance per month by ship type.

### 5b. Naval Combat

#### Research questions
- How did naval engagements work in this period? What determined the outcome? (ship type matchups, crew quality, weather gauge, gunnery, boarding)
- What were the major tactical considerations? (raking fire, crossing the T, boarding vs. stand-off gunnery, use of fireships)
- How did fortifications interact with naval forces? Could a fort reliably prevent a fleet from entering a harbor? What was the relative strength of ship guns vs. shore batteries?
- What were the typical casualty rates and damage profiles in naval engagements? How long did it take to repair battle damage?
- How did convoy systems work? What was the size and composition of a typical convoy escort? How effective were convoys against piracy?

#### Desired output
- A combat model framework: what factors determine the outcome of an engagement between two ships or fleets, and with what probabilities.
- Fort vs. ship interaction model: under what conditions can a fleet force a harbor, and at what cost?
- Convoy model: composition, cost, effectiveness against different threat levels.
- Damage and repair model: typical battle damage, repair time and cost, crew casualties as a function of engagement type.

### 5c. Land Warfare in the Caribbean

#### Research questions
- How did land warfare work in the Caribbean? What were the major types of military action? (siege of fortified port, amphibious assault, overland march, raid)
- What garrison sizes were typical for different settlement types?
- What was the role of fortifications? What did they cost to build and maintain? How effective were they?
- How did disease affect military campaigns? (Yellow fever, malaria, dysentery as major killers of European soldiers in tropical campaigns)
- What were the major land engagements of the period and what can we learn from them about the balance between attackers and defenders?

#### Desired output
- Garrison size and cost model by settlement type.
- Fortification types, costs, and defensive values.
- Amphibious assault model: what force ratio was needed to take a fortified port?
- Disease attrition model: expected losses per month for European troops in tropical conditions, and how this varied by season and acclimatization.

### Starting points for all of section 5
- Rodger, *The Command of the Ocean: A Naval History of Britain, 1649–1815* (2004).
- Gardiner (ed.), *The Line of Battle: The Sailing Warship, 1650–1840* (2004).
- Lavery, *The Ship of the Line* (2 vols., 1983–84).
- Davis, *The Rise of the English Shipping Industry* (1962).
- Goldenberg, *Shipbuilding in Colonial America* (1976).
- Harding, *Amphibious Warfare in the Eighteenth Century* (1991).
- McNeill, *Mosquito Empires: Ecology and War in the Greater Caribbean, 1620–1914* (2010) — essential on disease and military campaigns.
- Fortescue, *A History of the British Army* (1899–1930), relevant volumes.
- Chartrand, various Osprey volumes on Caribbean fortifications and colonial warfare.

---

## 6. Settlement Development and Colonial Growth

### What the simulation needs
A model of how settlements are founded, grow, develop economically, and decline. This is the "Anno layer" — the part of the simulation where colonies are built up over time through investment, immigration, and trade. The model must handle: founding new settlements, population growth (immigration and natural increase), economic development (what a settlement can produce changes as it grows), infrastructure investment, and decline (soil exhaustion, war damage, disease, economic obsolescence).

### Research questions
- How were new colonies founded in practice? What was the process and timeline from initial expedition to functioning settlement? What was the failure rate?
- What was the minimum viable settlement? (How many people, what supplies, what skills were needed to survive the first year?)
- How did settlements grow? What was the relative importance of immigration vs. natural increase? How did immigration vary by nationality and period?
- How did the economic character of a settlement change as it grew? (e.g., early subsistence farming → cash crop experimentation → plantation monoculture → mature colony with diversified economy and social stratification)
- What infrastructure did settlements build over time, and in what order? (cleared land, housing, warehouses, wharves, churches, fortifications, roads, mills, boiling houses)
- What caused settlements to decline or fail? How common was failure?
- How did the demographic structure of a settlement affect its development? (A sugar island with 90% enslaved population develops very differently from a New England town with free smallholders)
- What was the role of colonial charters, companies, and proprietors in directing development?

### Desired output
- A settlement development model: stages of growth with characteristic population levels, economic outputs, infrastructure, and requirements to advance to the next stage.
- Immigration data: annual immigration by nationality, destination, and period. Cost of transporting and establishing a settler.
- Settlement failure rates and causes.
- Infrastructure cost and construction time estimates.
- Demographic models for different colony types (plantation colony vs. settler colony vs. trading post vs. naval base).

### Starting points
- Dunn, *Sugar and Slaves* (1972). Detailed on the transformation of Barbados and Jamaica.
- Games, *Migration and the Origins of the English Atlantic World* (1999).
- Kupperman, *Providence Island, 1630–1641: The Other Puritan Colony* (1993) — a detailed study of a colony that failed.
- Galenson, *White Servitude in Colonial America* (1981).
- Berlin, *Many Thousands Gone: The First Two Centuries of Slavery in North America* (1998).
- Engerman and Higman, "The Demographic Structure of the Caribbean Slave Societies," in *General History of the Caribbean*, vol. 3.
- Watts, *The West Indies: Patterns of Development, Culture and Environmental Change since 1492* (1987).

---

## 7. Diplomacy and Inter-Faction Relations

### What the simulation needs
A model of how factions interact: alliances, wars, treaties, trade agreements, privateering commissions, and the informal relationships (smuggling networks, bribery, local truces that don't match metropolitan policy). The Caribbean was notable for having its own diplomatic dynamics that often diverged from European diplomacy — the phrase "no peace beyond the line" captured the principle that European treaties didn't necessarily apply in the Caribbean.

### Research questions
- How did the principle of "no peace beyond the line" actually work? When did European peace treaties apply in the Caribbean and when didn't they?
- What were the mechanisms of local diplomacy? (Governor-to-governor negotiation, local truces, trade agreements, marriage alliances)
- How did privateering commissions work legally and practically? What was the process for obtaining one, what were the terms, and how were prizes adjudicated?
- What was the role of bribery and corruption in colonial governance? How did governors enrich themselves and how did this affect their policy decisions?
- How did small colonies and non-state actors (pirates, smugglers, indigenous groups) navigate between the great powers?
- What triggered escalation from trade competition to open warfare in the Caribbean specifically?

### Desired output
- A diplomatic mechanics model: what actions factions can take toward each other (declare war, offer alliance, issue privateer commissions, impose embargo, grant trade concessions, bribe officials), what triggers them, and what effects they have.
- Relationship value system: what improves/damages relations between factions.
- The legal and practical framework for privateering.
- A model of "no peace beyond the line" and local diplomatic autonomy.

### Starting points
- Pares, *War and Trade in the West Indies* (1936).
- Haring, *The Buccaneers in the West Indies in the XVII Century* (1910).
- Lane, *Pillaging the Empire* (1998).
- Armitage and Braddick (eds.), *The British Atlantic World, 1500–1800* (2002).
- Pestana, *The English Conquest of Jamaica: Oliver Cromwell's Bid for Empire* (2017).

---

## 8. Exogenous Events and the European Context

### What the simulation needs
A model of events that originate outside the Caribbean but affect it. These include: European wars (which determine who is at war with whom, what military resources are deployed, and what trade restrictions change), metropolitan policy shifts (new tariffs, trade liberalization, colonial reorganization), technological changes (new ship types, improved sugar processing, new crops), and natural events (hurricanes, epidemics, earthquakes). These events are inputs to the simulation, not outputs — the player cannot prevent the War of Spanish Succession, but they must deal with its Caribbean consequences.

### Research questions
- What were the major European wars of 1600–1720 and what were their specific Caribbean manifestations? For each: dates, belligerents, Caribbean theaters of operation, major engagements, territorial outcomes, trade disruption effects.
- What major policy changes affected Caribbean trade, and when? (e.g., iterations of the Navigation Acts, Spanish convoy reforms, French Exclusif establishment and modifications)
- What new technologies or crops were introduced during the period, and what were their effects?
- What was the frequency and severity of hurricanes by region and season? What was a major hurricane's typical impact on a settlement?
- What were the major disease events and their demographic impacts?

### Desired output
- A master timeline of exogenous events, 1600–1720, with: date, type (war, policy, technology, natural disaster, epidemic), affected regions/factions, and estimated impact on the simulation systems (trade volumes, military deployments, population, production).
- For wars: a breakdown into phases relevant to the Caribbean, not just the European start/end dates.
- For hurricanes: a probability model (frequency per year by region) and impact model (damage to shipping, settlements, crops).

### Starting points
- Black, *European Warfare, 1660–1815* (1994).
- Harding, *The Dead Sea Squab* — Caribbean-specific military operations.
- Pares, *War and Trade in the West Indies* (1936).
- Mulcahy, *Hurricanes and Society in the British Greater Caribbean* (2008).
- McNeill, *Mosquito Empires* (2010).
- Schwartz (ed.), *Tropical Babylons: Sugar and the Making of the Atlantic World, 1450–1680* (2004).

---

## 9. Money, Finance, and Capital

### What the simulation needs
A model of how economic activity is financed and how wealth is accumulated and transferred. The Caribbean economy ran on a complex mix of specie (mostly Spanish silver), bills of exchange, barter, commodity money (tobacco was legal tender in Virginia), trade credit, and outright debt. The simulation needs to handle: a medium of exchange, capital investment in colonial development, trade financing, insurance, and the flow of wealth between colonies and metropoles.

### Research questions
- What currencies circulated in the Caribbean and at what exchange rates? How did Spanish pieces of eight become the de facto universal currency?
- How was trade financed? What was the role of bills of exchange, letters of credit, and trade credit? What were typical credit terms?
- What was the cost of capital in the colonies? Interest rates for loans, investment returns expected by plantation investors?
- How did maritime insurance work? What were premium rates by route, season, and wartime/peacetime? How did insurance affect the economics of trade and risk-taking?
- How did plantation investment work? Who provided the capital, on what terms, and how were profits repatriated?
- What was the fiscal structure of colonial governments? What revenues did they collect (port duties, head taxes, land taxes) and what did they spend on (fortifications, governor's salary, militia)?
- How did the Spanish bullion system work in practice? How was silver extracted, taxed (the quinto real), transported, and defended?

### Desired output
- A currency and exchange rate model for the simulation.
- A capital investment model: cost to establish different types of colonial enterprises, expected returns, financing mechanisms.
- Insurance rate tables by route and condition.
- Colonial fiscal model: revenues and expenditures for a typical settlement by size and type.
- Spanish bullion flow model: production volumes, tax rates, transport system, and leakage (smuggling, piracy).

### Starting points
- Zahedieh, *The Capital and the Colonies: London and the Atlantic Economy, 1660–1700* (2010).
- McCusker, *Money and Exchange in Europe and America, 1600–1775* (1978).
- Bakewell, *Silver Mining and Society in Colonial Mexico: Zacatecas, 1546–1700* (1971).
- TePaske, *A New World of Gold and Silver* (2010) — on bullion flows.
- Leonard and Pretel (eds.), *The Caribbean and the Atlantic World Economy: Circuits of Trade, Money, and Knowledge, 1650–1914* (2015).
- Pearce, *British Trade with Spanish America, 1763–1808* (2007).

---

## 10. Population, Labor, and Demographics

### What the simulation needs
A population model for each settlement: how many people, of what type (free European settlers, enslaved Africans, indigenous people, free people of color), how the population grows or shrinks, and what the population can do (labor supply, military potential, consumption demand). Population is the fundamental resource — it determines what a settlement can produce, how much it consumes, and how well it can defend itself.

### Research questions
- What were population levels for major Caribbean colonies at benchmark dates? Broken down by demographic category?
- What were birth, death, and net migration rates for each demographic category in different colony types? (Critically: enslaved populations in sugar colonies had negative natural growth — deaths exceeded births — requiring constant importation. Why, and by how much?)
- What was the volume and cost of the Atlantic slave trade by decade and destination?
- What was the volume and cost of European immigration by nationality, period, and destination?
- What was the cost of indentured servitude — passage, terms, and what happened after the term?
- How did labor supply constrain production? Are there documented cases where available land or capital couldn't be exploited due to labor shortages?
- What was the military potential of different population types? Who could be armed and mobilized for defense? (European settlers, buccaneers, militias, enslaved people in some circumstances)
- How did disease affect different populations differently? (European soldiers dying of yellow fever; African populations with some malaria resistance; indigenous populations devastated by European diseases)

### Desired output
- Population tables for major colonies at benchmark dates, by demographic category.
- Demographic rate models: birth, death, and migration rates by category and colony type.
- Slave trade volume and price data by decade and destination.
- European immigration volume and cost data.
- Labor productivity estimates: output per worker by good and colony type.
- Disease and mortality models: expected death rates for different populations in different environments, especially for newly arrived Europeans in the tropics.

### Starting points
- Eltis, *The Rise of African Slavery in the Americas* (2000).
- The Trans-Atlantic Slave Trade Database (slavevoyages.org) — comprehensive quantitative data.
- Engerman and Higman, "The Demographic Structure of the Caribbean Slave Societies," in *General History of the Caribbean*, vol. 3.
- Galenson, *White Servitude in Colonial America* (1981).
- Dunn, *Sugar and Slaves* (1972).
- McNeill, *Mosquito Empires* (2010) — essential on differential disease mortality.
- Games, *Migration and the Origins of the English Atlantic World* (1999).
- Berlin, *Many Thousands Gone* (1998).

---

## Research Methodology

### Prioritization
The sections above are roughly ordered by how foundational they are to the simulation. However, for a first prototype, the minimum viable research covers:
1. **Goods taxonomy** (section 3a) — what is being traded
2. **Production profiles** (section 3b) — who makes what
3. **Ship types** (section 5a) — what moves goods and fights
4. **Trade costs** (section 3c) — what it costs to move things
5. **Tariff regimes** (section 4) — what the rules are
6. **Map** (section 1) — where everything is

Everything else can be added incrementally as the simulation becomes more sophisticated.

### Data standards
- All prices in Spanish pieces of eight (reales de a ocho) as the common currency, with conversion notes for pounds sterling, guilders, and livres tournois where relevant.
- All weights in metric tons for comparability.
- All distances in nautical miles.
- All times in days (for voyages) or months (for construction/development).
- Sources cited at the level of specific pages, tables, or datasets within works — not just "see Sheridan 1974."

### Validation approach
The simulation should be able to reproduce known historical patterns as emergent behavior:
- Sugar dominates Caribbean exports by value (after bullion).
- The tobacco price collapse occurs when Virginia production scales.
- Smuggling concentrates at Dutch free ports when mercantilist restrictions create large price differentials.
- Caribbean sugar islands import food from North America because monoculture makes local food production uneconomical.
- Piracy concentrates on high-value, poorly-defended routes and declines when naval patrols increase.
- War disrupts trade and causes price spikes for imported goods in the colonies.

If the simulation reproduces these patterns without being hard-coded to do so, the underlying model is likely sound.
