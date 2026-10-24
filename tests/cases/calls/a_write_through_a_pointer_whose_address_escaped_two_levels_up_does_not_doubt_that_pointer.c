void *malloc(int n);
void free(void *p);
void opaque(int ****r);
int f(void) {
    int *p = malloc(4);
    int *q = malloc(8);
    int **pp = &p;
    int ***ppp = &pp;
    opaque(&ppp);
    **ppp = q;
    free(p);
    *q = 1;
    return 0;
}
