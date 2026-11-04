void *memcpy(void *d, void *s, int n);
int main(void) {
    int x = 0;
    int *px = &x;
    int *m[2];
    int **e = m;
    int **g = 0;
    memcpy(&g, &e, 8);
    *g = px;
    int *r = m[1];
    return r == 0;
}
