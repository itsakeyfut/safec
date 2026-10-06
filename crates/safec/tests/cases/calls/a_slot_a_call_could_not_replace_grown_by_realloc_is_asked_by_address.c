void *malloc(int n);
void *realloc(void *p, int n);
void release_ref(int **pp);
int use2(int **pp);

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
    int *b = a;
    release_ref(&b);
    int **h2 = realloc(h, 16);
    if (h2 == 0) {
        return 0;
    }
    int *c = *h2;
    return use2(&c);
}
