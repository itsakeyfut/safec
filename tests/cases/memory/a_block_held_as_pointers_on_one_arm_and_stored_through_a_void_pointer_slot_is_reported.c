void *malloc(int n);
void stored(void ** _Nonnull out, int c) {
    void *v = malloc(8);
    if (c) {
        int **b = v;
    }
    *out = v;
}
