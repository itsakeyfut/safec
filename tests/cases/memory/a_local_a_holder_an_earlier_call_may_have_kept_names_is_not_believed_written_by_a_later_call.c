void fill(int ***p);
void keep(int ***p);
void tick(void);
int f(int c) {
    int *m[1];
    int **h = m;
    int ***p = 0;
    if (c) p = &h;
    keep(p);
    tick();
    int *q = m[0];
    return q == 0;
}
