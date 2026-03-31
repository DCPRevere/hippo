use chrono::{Duration, Utc};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

pub struct GraphFixture {
    entities: Vec<FixtureEntity>,
    edges: Vec<FixtureEdge>,
    name_to_id: HashMap<String, String>,
}

struct FixtureEntity {
    id: String,
    name: String,
    entity_type: String,
}

struct FixtureEdge {
    subject_id: String,
    object_id: String,
    fact: String,
    relation_type: String,
    confidence: f32,
    salience: i64,
    valid_at: String,
    source_agents: String,
    memory_tier: String,
}

impl GraphFixture {
    pub fn build() -> Self {
        let mut fixture = Self {
            entities: Vec::new(),
            edges: Vec::new(),
            name_to_id: HashMap::new(),
        };
        fixture.create_entities();
        fixture.create_edges();
        fixture
    }

    pub fn entity_id(&self, name: &str) -> &str {
        self.name_to_id.get(name).unwrap_or_else(|| panic!("no entity named '{name}'"))
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    fn add_entity(&mut self, name: &str, entity_type: &str) -> String {
        let id = Uuid::new_v4().to_string();
        self.entities.push(FixtureEntity {
            id: id.clone(),
            name: name.to_string(),
            entity_type: entity_type.to_string(),
        });
        self.name_to_id.insert(name.to_string(), id.clone());
        id
    }

    fn add_edge(
        &mut self,
        subject: &str,
        object: &str,
        fact: &str,
        relation_type: &str,
        confidence: f32,
        salience: i64,
        days_ago: i64,
        source: &str,
        tier: &str,
    ) {
        let subject_id = self.name_to_id.get(subject).unwrap_or_else(|| panic!("no entity '{subject}'")).clone();
        let object_id = self.name_to_id.get(object).unwrap_or_else(|| panic!("no entity '{object}'")).clone();
        let valid_at = (Utc::now() - Duration::days(days_ago)).to_rfc3339();
        self.edges.push(FixtureEdge {
            subject_id,
            object_id,
            fact: fact.to_string(),
            relation_type: relation_type.to_string(),
            confidence,
            salience,
            valid_at,
            source_agents: source.to_string(),
            memory_tier: tier.to_string(),
        });
    }

    fn create_entities(&mut self) {
        // People (10)
        self.add_entity("Alice", "person");
        self.add_entity("Bob", "person");
        self.add_entity("Carol", "person");
        self.add_entity("David", "person");
        self.add_entity("Eve", "person");
        self.add_entity("Frank", "person");
        self.add_entity("Grace", "person");
        self.add_entity("Henry", "person");
        self.add_entity("Iris", "person");
        self.add_entity("Jack", "person");

        // More people (6)
        self.add_entity("Dr. Smith", "person");
        self.add_entity("Dr. Patel", "person");
        self.add_entity("Prof. Chen", "person");
        self.add_entity("Sarah", "person");
        self.add_entity("Tom", "person");
        self.add_entity("Principal", "person");

        // Organisations (8)
        self.add_entity("Acme Corp", "organization");
        self.add_entity("Beta Ltd", "organization");
        self.add_entity("Gamma Inc", "organization");
        self.add_entity("City Hospital", "organization");
        self.add_entity("First Bank", "organization");
        self.add_entity("Oxford University", "organization");
        self.add_entity("Stanford University", "organization");
        self.add_entity("TechVentures Fund", "organization");

        // Places (6)
        self.add_entity("London", "place");
        self.add_entity("Cambridge", "place");
        self.add_entity("Edinburgh", "place");
        self.add_entity("New York", "place");
        self.add_entity("San Francisco", "place");
        self.add_entity("Tokyo", "place");

        // Concepts (8)
        self.add_entity("Machine Learning", "concept");
        self.add_entity("Software Engineering", "concept");
        self.add_entity("Cardiology", "concept");
        self.add_entity("Diabetes", "concept");
        self.add_entity("Investment Strategy", "concept");
        self.add_entity("Quantum Computing", "concept");
        self.add_entity("Data Privacy", "concept");
        self.add_entity("Climate Science", "concept");

        // Events (4)
        self.add_entity("AI Conference 2025", "event");
        self.add_entity("Board Meeting Q1", "event");
        self.add_entity("Medical Symposium", "event");
        self.add_entity("Company Retreat", "event");

        // Additional entities for richer graph (8)
        self.add_entity("Project Phoenix", "concept");
        self.add_entity("Savings Account", "concept");
        self.add_entity("Mortgage", "concept");
        self.add_entity("Index Fund", "concept");
        self.add_entity("Blood Test Results", "concept");
        self.add_entity("Metformin", "concept");
        self.add_entity("Neural Networks Paper", "concept");
        self.add_entity("Patent Portfolio", "concept");
    }

    fn create_edges(&mut self) {
        // Employment relationships (12)
        self.add_edge("Alice", "Acme Corp", "Alice works at Acme Corp as a senior engineer", "WORKS_AT", 0.95, 3, 30, "agent1", "long_term");
        self.add_edge("Bob", "Acme Corp", "Bob works at Acme Corp as a product manager", "WORKS_AT", 0.9, 2, 45, "agent1", "long_term");
        self.add_edge("Carol", "Beta Ltd", "Carol is the CTO of Beta Ltd", "WORKS_AT", 0.95, 4, 20, "agent2", "long_term");
        self.add_edge("David", "Gamma Inc", "David works at Gamma Inc as a data scientist", "WORKS_AT", 0.85, 1, 60, "agent1", "long_term");
        self.add_edge("Eve", "City Hospital", "Eve works as a nurse at City Hospital", "WORKS_AT", 0.9, 2, 15, "agent3", "long_term");
        self.add_edge("Frank", "First Bank", "Frank is a financial analyst at First Bank", "WORKS_AT", 0.88, 1, 50, "agent1", "long_term");
        self.add_edge("Grace", "Oxford University", "Grace is a professor at Oxford University", "WORKS_AT", 0.95, 5, 10, "agent2", "long_term");
        self.add_edge("Henry", "Acme Corp", "Henry is the CEO of Acme Corp", "WORKS_AT", 0.98, 4, 40, "agent1", "long_term");
        self.add_edge("Dr. Smith", "City Hospital", "Dr. Smith practices medicine at City Hospital", "WORKS_AT", 0.95, 3, 35, "agent3", "long_term");
        self.add_edge("Dr. Patel", "City Hospital", "Dr. Patel is a cardiologist at City Hospital", "WORKS_AT", 0.92, 2, 25, "agent3", "long_term");
        self.add_edge("Prof. Chen", "Stanford University", "Prof. Chen teaches at Stanford University", "WORKS_AT", 0.9, 3, 55, "agent2", "long_term");
        self.add_edge("Tom", "TechVentures Fund", "Tom is a partner at TechVentures Fund", "WORKS_AT", 0.87, 1, 30, "agent1", "long_term");

        // Family relationships (8)
        self.add_edge("Alice", "Bob", "Alice and Bob are married", "MARRIED_TO", 0.99, 5, 80, "agent1", "long_term");
        self.add_edge("Alice", "Carol", "Alice and Carol are sisters", "SIBLING_OF", 0.95, 3, 75, "agent1", "long_term");
        self.add_edge("David", "Eve", "David and Eve are married", "MARRIED_TO", 0.97, 4, 70, "agent2", "long_term");
        self.add_edge("Frank", "Grace", "Frank is Grace's brother", "SIBLING_OF", 0.9, 2, 65, "agent2", "long_term");
        self.add_edge("Henry", "Iris", "Henry and Iris are married", "MARRIED_TO", 0.96, 3, 60, "agent1", "long_term");
        self.add_edge("Alice", "Sarah", "Alice is Sarah's mother", "PARENT_OF", 0.98, 4, 50, "agent1", "long_term");
        self.add_edge("Bob", "Sarah", "Bob is Sarah's father", "PARENT_OF", 0.98, 4, 50, "agent1", "long_term");
        self.add_edge("Jack", "Tom", "Jack and Tom are brothers", "SIBLING_OF", 0.85, 1, 40, "agent2", "long_term");

        // Location relationships (12)
        self.add_edge("Alice", "London", "Alice lives in London", "LIVES_IN", 0.95, 3, 30, "agent1", "long_term");
        self.add_edge("Bob", "London", "Bob lives in London", "LIVES_IN", 0.93, 2, 30, "agent1", "long_term");
        self.add_edge("Carol", "Cambridge", "Carol lives in Cambridge", "LIVES_IN", 0.9, 2, 25, "agent2", "long_term");
        self.add_edge("David", "Edinburgh", "David lives in Edinburgh", "LIVES_IN", 0.88, 1, 40, "agent1", "long_term");
        self.add_edge("Eve", "Edinburgh", "Eve lives in Edinburgh", "LIVES_IN", 0.88, 1, 40, "agent2", "long_term");
        self.add_edge("Grace", "Cambridge", "Grace resides in Cambridge near the university", "LIVES_IN", 0.92, 3, 15, "agent2", "long_term");
        self.add_edge("Acme Corp", "London", "Acme Corp is headquartered in London", "BASED_IN", 0.97, 4, 60, "agent1", "long_term");
        self.add_edge("Beta Ltd", "Cambridge", "Beta Ltd is based in Cambridge", "BASED_IN", 0.95, 3, 45, "agent2", "long_term");
        self.add_edge("Gamma Inc", "New York", "Gamma Inc has its main office in New York", "BASED_IN", 0.9, 2, 55, "agent1", "long_term");
        self.add_edge("City Hospital", "Edinburgh", "City Hospital is located in Edinburgh", "BASED_IN", 0.98, 5, 70, "agent3", "long_term");
        self.add_edge("Frank", "New York", "Frank lives in New York", "LIVES_IN", 0.85, 1, 35, "agent1", "long_term");
        self.add_edge("Jack", "San Francisco", "Jack lives in San Francisco", "LIVES_IN", 0.87, 1, 20, "agent1", "long_term");

        // Education relationships (10)
        self.add_edge("Alice", "Oxford University", "Alice studied computer science at Oxford University", "STUDIED_AT", 0.92, 3, 85, "agent1", "long_term");
        self.add_edge("Bob", "Oxford University", "Bob graduated from Oxford University with a degree in engineering", "GRADUATED_FROM", 0.9, 2, 85, "agent1", "long_term");
        self.add_edge("Carol", "Stanford University", "Carol earned her PhD at Stanford University", "GRADUATED_FROM", 0.93, 3, 80, "agent2", "long_term");
        self.add_edge("David", "Oxford University", "David studied mathematics at Oxford University", "STUDIED_AT", 0.88, 2, 75, "agent1", "long_term");
        self.add_edge("Grace", "Stanford University", "Grace completed her postdoc at Stanford University", "STUDIED_AT", 0.91, 2, 70, "agent2", "long_term");
        self.add_edge("Prof. Chen", "Oxford University", "Prof. Chen was a visiting scholar at Oxford University", "STUDIED_AT", 0.85, 1, 60, "agent2", "long_term");
        self.add_edge("Henry", "Cambridge", "Henry studied business at Cambridge", "STUDIED_AT", 0.8, 1, 55, "agent1", "long_term");
        self.add_edge("Iris", "Stanford University", "Iris studied biology at Stanford", "STUDIED_AT", 0.82, 1, 50, "agent2", "long_term");
        self.add_edge("Frank", "Oxford University", "Frank graduated from Oxford with a finance degree", "GRADUATED_FROM", 0.86, 2, 65, "agent2", "long_term");
        self.add_edge("Sarah", "Oxford University", "Sarah is currently studying at Oxford University", "STUDIED_AT", 0.8, 1, 5, "agent1", "working");

        // Medical relationships (10)
        self.add_edge("Principal", "Dr. Smith", "Principal is treated by Dr. Smith", "TREATED_BY", 0.9, 3, 30, "agent3", "long_term");
        self.add_edge("Principal", "Diabetes", "Principal has been diagnosed with type 2 diabetes", "DIAGNOSED_WITH", 0.95, 4, 60, "agent3", "long_term");
        self.add_edge("Dr. Smith", "Dr. Patel", "Dr. Smith referred a patient to Dr. Patel for cardiology", "REFERRED_TO", 0.88, 2, 20, "agent3", "long_term");
        self.add_edge("Principal", "Metformin", "Principal takes metformin for diabetes management", "PRESCRIBED", 0.92, 3, 45, "agent3", "long_term");
        self.add_edge("Principal", "Blood Test Results", "Principal had a blood test with normal results", "HAS_RESULT", 0.85, 1, 10, "agent3", "working");
        self.add_edge("Dr. Patel", "Cardiology", "Dr. Patel specializes in cardiology", "SPECIALIZES_IN", 0.95, 3, 40, "agent3", "long_term");
        self.add_edge("Eve", "Dr. Smith", "Eve assists Dr. Smith in the clinic", "ASSISTS", 0.82, 1, 15, "agent3", "long_term");
        self.add_edge("David", "Dr. Patel", "David is being treated by Dr. Patel for a heart condition", "TREATED_BY", 0.87, 2, 25, "agent3", "long_term");
        self.add_edge("Alice", "Dr. Patel", "Alice recommended Dr. Patel after her own checkup", "RECOMMENDED", 0.8, 1, 18, "agent1", "long_term");
        self.add_edge("City Hospital", "Medical Symposium", "City Hospital hosted the Medical Symposium", "HOSTED", 0.9, 2, 12, "agent3", "long_term");

        // Financial relationships (10)
        self.add_edge("Principal", "First Bank", "Principal has a savings account at First Bank", "ACCOUNTS_AT", 0.92, 3, 50, "agent1", "long_term");
        self.add_edge("Principal", "Savings Account", "Principal maintains a savings account", "OWNS", 0.88, 2, 50, "agent1", "long_term");
        self.add_edge("Principal", "Mortgage", "Principal has a mortgage with City Credit Union", "OWNS", 0.9, 2, 55, "agent1", "long_term");
        self.add_edge("Principal", "Index Fund", "Principal invests monthly in an index fund through Vanguard", "INVESTS_IN", 0.87, 2, 40, "agent1", "long_term");
        self.add_edge("Alice", "First Bank", "Alice has a joint account at First Bank", "ACCOUNTS_AT", 0.85, 1, 45, "agent1", "long_term");
        self.add_edge("Frank", "Investment Strategy", "Frank developed an investment strategy for clients", "EXPERT_IN", 0.82, 2, 35, "agent1", "long_term");
        self.add_edge("Tom", "TechVentures Fund", "Tom manages investments at TechVentures Fund", "MANAGES", 0.9, 3, 30, "agent1", "long_term");
        self.add_edge("Tom", "Acme Corp", "TechVentures Fund invested in Acme Corp", "INVESTS_IN", 0.85, 2, 25, "agent1", "long_term");
        self.add_edge("First Bank", "London", "First Bank is headquartered in London", "BASED_IN", 0.95, 3, 60, "agent1", "long_term");
        self.add_edge("Henry", "Patent Portfolio", "Henry oversees Acme Corp's patent portfolio", "MANAGES", 0.8, 1, 20, "agent1", "long_term");

        // Social / knows relationships (12)
        self.add_edge("Alice", "David", "Alice knows David from university", "KNOWS", 0.85, 2, 70, "agent1", "long_term");
        self.add_edge("Bob", "Carol", "Bob and Carol met at a tech conference", "KNOWS", 0.8, 1, 50, "agent2", "long_term");
        self.add_edge("Carol", "Prof. Chen", "Carol collaborates with Prof. Chen on research", "COLLABORATES_WITH", 0.88, 3, 30, "agent2", "long_term");
        self.add_edge("David", "Frank", "David and Frank are close friends", "FRIENDS_WITH", 0.9, 2, 55, "agent1", "long_term");
        self.add_edge("Grace", "Prof. Chen", "Grace and Prof. Chen co-authored a paper on machine learning", "COLLABORATES_WITH", 0.92, 4, 15, "agent2", "long_term");
        self.add_edge("Henry", "Tom", "Henry introduced Tom to invest in Acme Corp", "KNOWS", 0.82, 1, 30, "agent1", "long_term");
        self.add_edge("Iris", "Eve", "Iris and Eve are friends from college", "FRIENDS_WITH", 0.85, 2, 45, "agent2", "long_term");
        self.add_edge("Jack", "Alice", "Jack met Alice at the company retreat", "MET_AT", 0.75, 1, 10, "agent1", "working");
        self.add_edge("Alice", "Company Retreat", "Alice attended the company retreat", "ATTENDED", 0.85, 2, 10, "agent1", "working");
        self.add_edge("Jack", "Company Retreat", "Jack attended the company retreat", "ATTENDED", 0.82, 1, 10, "agent1", "working");
        self.add_edge("Carol", "AI Conference 2025", "Carol presented at the AI Conference 2025", "ATTENDED", 0.9, 3, 8, "agent2", "working");
        self.add_edge("Prof. Chen", "AI Conference 2025", "Prof. Chen gave a keynote at the AI Conference 2025", "ATTENDED", 0.92, 3, 8, "agent2", "working");

        // Research / expertise relationships (12)
        self.add_edge("Alice", "Software Engineering", "Alice is an expert in software engineering", "EXPERT_IN", 0.9, 3, 30, "agent1", "long_term");
        self.add_edge("Alice", "Machine Learning", "Alice has been studying machine learning", "INTERESTED_IN", 0.75, 1, 15, "agent1", "working");
        self.add_edge("Carol", "Machine Learning", "Carol is an expert in machine learning", "EXPERT_IN", 0.95, 5, 25, "agent2", "long_term");
        self.add_edge("David", "Machine Learning", "David applies machine learning in his data science work", "EXPERT_IN", 0.85, 2, 40, "agent1", "long_term");
        self.add_edge("Grace", "Quantum Computing", "Grace researches quantum computing", "EXPERT_IN", 0.9, 4, 10, "agent2", "long_term");
        self.add_edge("Prof. Chen", "Machine Learning", "Prof. Chen is a leading researcher in machine learning", "EXPERT_IN", 0.97, 5, 20, "agent2", "long_term");
        self.add_edge("Grace", "Neural Networks Paper", "Grace published a paper on neural networks", "AUTHORED", 0.88, 3, 12, "agent2", "long_term");
        self.add_edge("Prof. Chen", "Neural Networks Paper", "Prof. Chen co-authored the neural networks paper with Grace", "AUTHORED", 0.88, 3, 12, "agent2", "long_term");
        self.add_edge("Carol", "Data Privacy", "Carol leads research on data privacy", "EXPERT_IN", 0.87, 2, 20, "agent2", "long_term");
        self.add_edge("Bob", "Project Phoenix", "Bob manages Project Phoenix at Acme Corp", "MANAGES", 0.85, 2, 15, "agent1", "long_term");
        self.add_edge("Alice", "Project Phoenix", "Alice is the lead engineer on Project Phoenix", "WORKS_ON", 0.88, 3, 15, "agent1", "long_term");
        self.add_edge("Henry", "Board Meeting Q1", "Henry chaired the Q1 board meeting", "ATTENDED", 0.9, 2, 30, "agent1", "long_term");

        // Cross-domain edges (14)
        self.add_edge("Dr. Smith", "Diabetes", "Dr. Smith treats patients with diabetes", "TREATS", 0.9, 3, 40, "agent3", "long_term");
        self.add_edge("Acme Corp", "Machine Learning", "Acme Corp is investing in machine learning research", "INVESTS_IN", 0.82, 2, 20, "agent1", "long_term");
        self.add_edge("Beta Ltd", "Data Privacy", "Beta Ltd develops data privacy solutions", "DEVELOPS", 0.88, 3, 25, "agent2", "long_term");
        self.add_edge("Gamma Inc", "Climate Science", "Gamma Inc funds climate science research", "FUNDS", 0.8, 1, 35, "agent1", "long_term");
        self.add_edge("Oxford University", "Cambridge", "Oxford University has a partnership with Cambridge institutions", "PARTNERS_WITH", 0.75, 1, 50, "agent2", "long_term");
        self.add_edge("Stanford University", "Machine Learning", "Stanford University is a leading center for machine learning", "SPECIALIZES_IN", 0.95, 5, 30, "agent2", "long_term");
        self.add_edge("TechVentures Fund", "Beta Ltd", "TechVentures Fund invested in Beta Ltd", "INVESTS_IN", 0.88, 2, 20, "agent1", "long_term");
        self.add_edge("Iris", "Climate Science", "Iris studies the intersection of biology and climate science", "INTERESTED_IN", 0.78, 1, 15, "agent2", "working");
        self.add_edge("Jack", "Acme Corp", "Jack recently joined Acme Corp", "WORKS_AT", 0.8, 1, 5, "agent1", "working");
        self.add_edge("Sarah", "Acme Corp", "Sarah is interning at Acme Corp", "WORKS_AT", 0.75, 1, 3, "agent1", "working");
        self.add_edge("Dr. Smith", "Medical Symposium", "Dr. Smith presented research at the Medical Symposium", "ATTENDED", 0.88, 2, 12, "agent3", "long_term");
        self.add_edge("Eve", "Medical Symposium", "Eve helped organize the Medical Symposium", "ORGANIZED", 0.82, 1, 12, "agent3", "long_term");
        self.add_edge("Henry", "Investment Strategy", "Henry applies investment strategies for Acme Corp growth", "APPLIES", 0.8, 1, 25, "agent1", "long_term");
        self.add_edge("Frank", "First Bank", "Frank manages client portfolios at First Bank", "MANAGES", 0.85, 2, 30, "agent1", "long_term");

        // Low-confidence / varied edges for testing filters (10)
        self.add_edge("Jack", "Quantum Computing", "Jack is interested in quantum computing", "INTERESTED_IN", 0.55, 0, 2, "agent1", "working");
        self.add_edge("Tom", "Climate Science", "Tom might be interested in climate science investments", "INTERESTED_IN", 0.5, 0, 1, "agent1", "working");
        self.add_edge("Iris", "Tokyo", "Iris visited Tokyo last summer", "VISITED", 0.7, 0, 60, "agent2", "long_term");
        self.add_edge("Frank", "San Francisco", "Frank travels to San Francisco frequently", "VISITS", 0.65, 0, 30, "agent1", "long_term");
        self.add_edge("Grace", "London", "Grace visited London for a conference", "VISITED", 0.7, 1, 20, "agent2", "long_term");
        self.add_edge("Bob", "Edinburgh", "Bob visited Edinburgh for a project review", "VISITED", 0.6, 0, 15, "agent1", "working");
        self.add_edge("Alice", "AI Conference 2025", "Alice registered for the AI Conference 2025", "REGISTERED_FOR", 0.7, 0, 5, "agent1", "working");
        self.add_edge("David", "New York", "David frequently travels to New York for work", "VISITS", 0.75, 1, 20, "agent1", "long_term");
        self.add_edge("Henry", "San Francisco", "Henry visits San Francisco for board meetings", "VISITS", 0.72, 1, 25, "agent1", "long_term");
        self.add_edge("Carol", "London", "Carol commutes to London occasionally", "VISITS", 0.65, 0, 10, "agent2", "working");
    }

    pub fn to_seed_json(&self) -> serde_json::Value {
        let entities: Vec<serde_json::Value> = self.entities.iter().map(|e| {
            json!({
                "id": e.id,
                "name": e.name,
                "entity_type": e.entity_type,
                "resolved": true
            })
        }).collect();

        let edges: Vec<serde_json::Value> = self.edges.iter().map(|e| {
            json!({
                "subject_id": e.subject_id,
                "object_id": e.object_id,
                "fact": e.fact,
                "relation_type": e.relation_type,
                "confidence": e.confidence,
                "salience": e.salience,
                "valid_at": e.valid_at,
                "source_agents": e.source_agents,
                "memory_tier": e.memory_tier
            })
        }).collect();

        json!({
            "entities": entities,
            "edges": edges
        })
    }
}
