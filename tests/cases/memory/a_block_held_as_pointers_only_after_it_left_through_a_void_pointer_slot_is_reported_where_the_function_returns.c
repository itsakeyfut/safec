void *malloc(int n);
void stored(void ** _Nonnull out) {
    void *v = malloc(8);
    *out = v;
    int **b = v;
}
