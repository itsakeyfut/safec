void *malloc(int n);
void free(void *p);
void put(int ** _Nonnull pp) {
    for (int i = 0; i < 2; i++) {
        int *x = malloc(4);
        if (x == 0) {
            return;
        }
        if (i == 0) {
            *pp = x;
        } else {
            free(x);
        }
    }
}
