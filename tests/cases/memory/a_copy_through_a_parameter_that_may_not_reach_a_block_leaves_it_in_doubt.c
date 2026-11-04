void *malloc(int n);
void *memcpy(void *d, void *s, int n);

int f(int ***t, int ***s, int **y) {
    int **m = malloc(8);
    if (m == 0) {
        return 0;
    }
    *s = m;
    memcpy(*t, y, 8);
    int *q = *m;
    return q != 0;
}
