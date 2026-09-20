void *malloc(int n);
void free(void *p);
void opaque(int ***r);
int f(void) {
    int *p = malloc(4);
    int *q = malloc(8);
    int **pp;
    int ***ppp = &pp;
    pp = &p;
    opaque(ppp);
    *pp = q;
    free(p);
    *q = 1;
    return 0;
}
