void *memcpy(void *d, void *s, int n);
int f(void) {
    int x = 1;
    int *z;
    int **pz = &z;
    pz = pz + 1;
    int ***h = &pz;
    int *px = &x;
    memcpy(*h, &px, 8);
    int *q = z;
    return q != 0;
}
