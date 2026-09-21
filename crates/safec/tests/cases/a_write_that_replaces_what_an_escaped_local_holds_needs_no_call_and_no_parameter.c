void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int *r = p;
    int **pp = &p;
    int ***ppp = &pp;
    int **qq = *ppp;
    *qq = malloc(8);
    free(p);
    *r = 1;
    return 0;
}
