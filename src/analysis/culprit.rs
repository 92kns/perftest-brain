use crate::api::pushlog::Commit;

/// A commit ranked by its relevance to a specific regressed test.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RankedCommit {
    #[serde(flatten)]
    pub commit: Commit,
    pub score: u32,
    pub matched_areas: Vec<String>,
}

struct CodeArea {
    name: &'static str,
    path_prefixes: &'static [&'static str],
}

// Maps test/suite name patterns to relevant code areas
struct TestMapping {
    keywords: &'static [&'static str],
    areas: &'static [&'static str],
}

static TEST_MAPPINGS: &[TestMapping] = &[
    TestMapping {
        keywords: &[
            "speedometer",
            "todomvc",
            "react",
            "angular",
            "ember",
            "js-bench",
            "javascript",
        ],
        areas: &["js-engine", "layout", "css-engine"],
    },
    TestMapping {
        keywords: &["startup", "sessionrestore", "newtab", "about:home", "cold"],
        areas: &["startup", "browser-frontend"],
    },
    TestMapping {
        keywords: &["pageload", "tp6", "navigation", "load", "network"],
        areas: &["networking", "layout", "js-engine"],
    },
    TestMapping {
        keywords: &["memory", "awsy", "heap", "gc"],
        areas: &["memory", "js-engine"],
    },
    TestMapping {
        keywords: &["android", "fenix", "geckoview", "focus"],
        areas: &["android", "mobile"],
    },
    TestMapping {
        keywords: &["css", "paint", "render", "reflow", "layout"],
        areas: &["layout", "css-engine"],
    },
];

static CODE_AREAS: &[CodeArea] = &[
    CodeArea {
        name: "SpiderMonkey / JS engine",
        path_prefixes: &["js/src/", "js/public/"],
    },
    CodeArea {
        name: "Layout / rendering",
        path_prefixes: &["layout/", "dom/", "gfx/", "view/", "accessible/"],
    },
    CodeArea {
        name: "CSS engine",
        path_prefixes: &["servo/", "layout/style/", "dom/css/"],
    },
    CodeArea {
        name: "Networking",
        path_prefixes: &["netwerk/", "security/manager/", "modules/libpref/"],
    },
    CodeArea {
        name: "Startup / initialization",
        path_prefixes: &[
            "toolkit/components/startup/",
            "toolkit/components/places/",
            "browser/app/",
            "xpcom/base/",
            "xpcom/io/",
        ],
    },
    CodeArea {
        name: "Browser frontend",
        path_prefixes: &[
            "browser/components/",
            "browser/base/",
            "toolkit/components/extensions/",
        ],
    },
    CodeArea {
        name: "Memory management",
        path_prefixes: &["memory/", "xpcom/base/", "js/src/gc/"],
    },
    CodeArea {
        name: "Android / mobile",
        path_prefixes: &["mobile/android/", "geckoview/", "mobile/locales/"],
    },
    CodeArea {
        name: "Taskcluster / CI",
        path_prefixes: &["taskcluster/", ".taskcluster.yml"],
    },
    CodeArea {
        name: "Performance tests",
        path_prefixes: &[
            "testing/raptor/",
            "testing/mozperftest/",
            "testing/performance/",
            "testing/talos/",
            "testing/awsy/",
        ],
    },
];

fn relevant_areas(suite: &str, test: &str) -> Vec<usize> {
    let combined = format!("{suite} {test}").to_lowercase();
    let mut area_indices = std::collections::HashSet::new();

    for mapping in TEST_MAPPINGS {
        if mapping.keywords.iter().any(|kw| combined.contains(kw)) {
            for area_name in mapping.areas {
                if let Some(idx) = CODE_AREAS
                    .iter()
                    .position(|a| a.name.to_lowercase().contains(area_name))
                {
                    area_indices.insert(idx);
                }
            }
        }
    }

    if area_indices.is_empty() {
        // No specific match — all areas are relevant
        (0..CODE_AREAS.len()).collect()
    } else {
        let mut v: Vec<usize> = area_indices.into_iter().collect();
        v.sort_unstable();
        v
    }
}

fn score_commit(commit: &Commit, area_indices: &[usize]) -> (u32, Vec<String>) {
    if commit.is_noise {
        return (0, vec![]);
    }

    let mut matched = Vec::new();

    for &idx in area_indices {
        let area = &CODE_AREAS[idx];
        let touches = commit.files.iter().any(|f| {
            area.path_prefixes
                .iter()
                .any(|prefix| f.starts_with(prefix))
        });
        if touches {
            matched.push(area.name.to_owned());
        }
    }

    let mut score = (matched.len() as u32) * 10;
    if commit.bug_id.is_some() {
        score += 2;
    }

    (score, matched)
}

/// Rank commits in the regression window by relevance to the regressed test.
pub fn rank_commits(commits: Vec<Commit>, suite: &str, test: &str) -> Vec<RankedCommit> {
    let areas = relevant_areas(suite, test);

    let mut ranked: Vec<RankedCommit> = commits
        .into_iter()
        .map(|c| {
            let (score, matched_areas) = score_commit(&c, &areas);
            RankedCommit {
                commit: c,
                score,
                matched_areas,
            }
        })
        .collect();

    ranked.sort_by_key(|c| std::cmp::Reverse(c.score));
    ranked
}

/// Format ranked commits as a human-readable report.
pub fn format_ranked(commits: &[RankedCommit], suite: &str, test: &str) -> String {
    let substantive: Vec<_> = commits.iter().filter(|c| !c.commit.is_noise).collect();
    let noise_count = commits.iter().filter(|c| c.commit.is_noise).count();

    let mut out = String::new();
    out.push_str(&format!(
        "Regression window: {} commits ({} substantive, {} noise/l10n)\n",
        commits.len(),
        substantive.len(),
        noise_count
    ));
    out.push_str(&format!("Regressed test: {suite} / {test}\n\n"));

    if substantive.is_empty() {
        out.push_str("No substantive commits in window — may be infrastructure or intermittent.\n");
        return out;
    }

    out.push_str("Ranked by relevance to regressed test:\n");
    for c in &substantive {
        let indicator = if c.score > 0 {
            "  [SUSPECT]"
        } else {
            "  [  low  ]"
        };
        out.push_str(&format!(
            "{indicator} {}  {}\n",
            c.commit.short_node, c.commit.short_desc
        ));
        if !c.matched_areas.is_empty() {
            out.push_str(&format!(
                "             touches: {}\n",
                c.matched_areas.join(", ")
            ));
        }
        if !c.commit.files.is_empty() {
            let shown: Vec<_> = c.commit.files.iter().take(3).collect();
            let extra = c.commit.files.len().saturating_sub(3);
            let files_str = shown
                .iter()
                .map(|f| f.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if extra > 0 {
                format!(" (+{extra} more)")
            } else {
                String::new()
            };
            out.push_str(&format!("             files:   {files_str}{suffix}\n"));
        }
    }

    if let Some(top) = substantive.first() {
        out.push('\n');
        if top.score > 0 {
            let bug = top.commit.bug_id.as_deref().unwrap_or("unknown");
            out.push_str(&format!(
                "Top suspect: bug {bug} — {}\n",
                top.commit.short_desc
            ));
        } else if substantive.len() == 1 {
            out.push_str(&format!(
                "Only substantive commit: {} — {}\n(No direct file match, but only candidate in window)\n",
                top.commit.short_node, top.commit.short_desc
            ));
        }
    }

    out
}
