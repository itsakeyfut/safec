void keep(int ****p);
void tick(void);
int f(int c) {
    int *a;
    int **m;
    int **n;
    int ***h = &m;
    keep(&h);
    if (c) h = &n;
    m = &a;
    tick();
    return a == 0;
}
