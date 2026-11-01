void *memcpy(void *d, void *s, int n);

int main(void) {
    int x = 0;
    int *m[2];
    int **e = m + 1;
    int **f;
    memcpy(&f, &e, 8);
    *f = &x;
    int *r = m[0];
    return r == 0;
}
