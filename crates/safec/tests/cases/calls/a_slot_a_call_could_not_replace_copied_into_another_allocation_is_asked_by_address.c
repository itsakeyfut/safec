void *malloc(int n);
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
    int **g = malloc(8);
    if (g == 0) {
        return 0;
    }
    *h = a;
    int *b = a;
    release_ref(&b);
    *g = *h;
    int *c = *g;
    return use2(&c);
}
