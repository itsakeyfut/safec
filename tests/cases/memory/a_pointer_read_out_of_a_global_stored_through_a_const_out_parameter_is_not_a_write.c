const char *G;
const char **get2(void);
void *malloc(unsigned long long n);
void keep(const char **b);
void stored_from_a_global(const char **out) {
    if (out) {
        *out = G;
    }
}
void stored_beside_a_table_handed_on(const char **out) {
    const char **b = malloc(sizeof(const char *));
    if (b) {
        *b = "ab";
        keep(b);
    }
    if (out) {
        *out = G;
    }
}
void stored_address_of_a_global(const char ***out) {
    if (out) {
        *out = &G;
    }
}
void stored_a_call_result(const char ***out) {
    if (out) {
        *out = get2();
    }
}
