void *malloc(int n);
void stored(void ** _Nonnull out) {
    int **b = malloc(8);
    *out = b;
}
