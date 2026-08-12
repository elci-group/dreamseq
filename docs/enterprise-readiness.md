# Enterprise Readiness

## Target customer

The initial enterprise customer is an engineering-platform or developer-
productivity team operating AI coding agents across multiple harnesses. The
buyer needs evidence about rework, model failures, tool gaps, and workflow
friction before standardizing agent workflows.

Dreamseq is not positioned as application telemetry, employee surveillance,
or a calibrated productivity score. It is a privacy-conscious decision-support
system for improving AI-assisted engineering systems.

## Pilot boundary

Start with one team, two harnesses, and 7–14 days of logs. Use local-only mode
first. Enable remote analysis only after the customer approves the data flow.
The pilot produces versioned anthologies, ingestion reports, JSON summaries,
and a before/after measurement review.

## Customer data controls

Customers own their source logs and generated artifacts. They must define:

- approved log paths and repositories;
- retention and deletion periods;
- who may read anthologies and `.dreams` outputs;
- whether remote analysis is permitted;
- approved inference providers and regions;
- incident and credential-rotation contacts.

Dreamseq does not currently provide hosted RBAC, SSO, centralized retention
enforcement, or a formal DPA. These are procurement requirements for a managed
enterprise service, not current product claims.

## Readiness gates

An enterprise pilot is ready when the customer has a named owner, approved
data flow, rollback path, measurable baseline, and success review date. A paid
production rollout additionally requires legal terms, support commitments,
release provenance, and a documented retention/deletion process.
