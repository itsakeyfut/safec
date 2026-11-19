void keep(int ****p);
void tick(void);
int f(void) {
    int *a;
    int **m;
    int ***h = &m;
    keep(&h);
    h = 0;
    m = &a;
    tick();
    return a == 0;
}
