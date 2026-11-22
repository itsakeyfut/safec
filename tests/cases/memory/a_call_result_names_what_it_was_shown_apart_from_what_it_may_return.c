void *malloc(unsigned long long n);
void keep(const char **b);
const char **get(void);
char **get_writable(void);
void stored_beside_a_table_handed_on(const char ***out) {
    const char **b = malloc(sizeof(const char *));
    if (b) {
        *b = "ab";
        keep(b);
    }
    if (out) {
        *out = get();
    }
}
void written_through_what_a_result_may_be(void) {
    const char **b = malloc(sizeof(const char *));
    if (!b) {
        return;
    }
    *b = "ab";
    keep(b);
    char **r = get_writable();
    if (r && *r) {
        **r = 'x';
    }
}
