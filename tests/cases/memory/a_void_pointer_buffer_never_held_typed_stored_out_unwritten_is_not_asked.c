void *malloc(int n);
void stored(void ** _Nonnull out, int n) {
    void *p = malloc(n);
    *out = p;
}
