void *malloc(int n);
void release_ref(int **pp);
int use2(int **pp);
void keep(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int **h = malloc(8);
    if (h == 0) {
        return 0;
    }
    *h = a;
    int *z = malloc(4);
    if (z == 0) {
        return 0;
    }
    keep(&z);
    int *b = a;
    release_ref(&b);
    int *c = *h;
    c = z;
    return use2(&c);
}
