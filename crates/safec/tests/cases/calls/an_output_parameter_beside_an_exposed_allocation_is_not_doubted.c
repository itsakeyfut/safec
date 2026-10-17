void *malloc(int n);
void free(void *p);
void get(int **pp);
void keep(int *p);
int f(void) {
    int *a = malloc(4);
    if (!a) return 0;
    keep(a);
    int *p;
    get(&p);
    if (!p) return 0;
    *p = 1;
    return 0;
}
