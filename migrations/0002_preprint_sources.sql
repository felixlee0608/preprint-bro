-- Extend papers for multiple preprint servers: arXiv (the original),
-- bioRxiv and medRxiv. `arxiv_id` stays the canonical key: an arXiv id or
-- a DOI (bioRxiv/medRxiv papers are identified by their DOI, e.g.
-- 10.1101/2024.06.28.601293). `source` records which server the paper
-- came from so aliases, links and figures resolve correctly.
-- `provider` records which LLM provider built the course (see config.rs).

alter table papers add column source text not null default 'arxiv';
alter table courses add column provider text;
