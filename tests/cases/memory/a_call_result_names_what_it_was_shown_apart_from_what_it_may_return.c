void *malloc(unsigned long long n);
void keep(const char **b);
void keep_slot(const char **s);
void take(void *v);
const char **get(void);
char **get_writable(void);
void keep_const(const char **p);
void read_const_void(const void *p);
void *get_any(void);
void sink(void *p);
void keep_view(const char *const *p);
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
void handed_where_one_path_showed_the_table(int c) {
    const char **b = malloc(sizeof(const char *));
    if (!b) {
        return;
    }
    *b = "ab";
    keep(b);
    const char **r = b;
    if (c) {
        r = get();
    }
    take(r);
}
void stored_beside_a_local_handed_on(const char ***out) {
    const char *s = "ab";
    keep_slot(&s);
    if (out) {
        *out = get();
    }
}
void handed_as_void_beside_a_literal_kept(void) {
    const char *s = "ab";
    keep_view(&s);
    void *r = get_any();
    sink(r);
}
void handed_as_const_void_beside_a_literal_kept(void) {
    const char *s = 0;
    keep_const(&s);
    s = "ab";
    read_const_void(get_writable());
}
