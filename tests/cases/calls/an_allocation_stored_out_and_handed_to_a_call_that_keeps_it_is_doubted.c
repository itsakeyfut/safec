void *malloc(int n);
void append(int **list, int *n);
void put(int ** _Nonnull out, int ** _Nonnull list) {
    int *n = malloc(4);
    if (n == 0) {
        return;
    }
    *out = n;
    append(list, n);
}
