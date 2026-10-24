void *malloc(int n);
void free(void *p);
void opaque(int **r);
int f(void) {
    int *p = malloc(4);
    int *q = malloc(8);
    int **pp = &p;
    *pp = q;
    opaque(pp);
    free(p);
    *q = 1;
    return 0;
}
