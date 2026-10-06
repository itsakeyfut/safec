void *malloc(int n);

int f(void) {
    int *p = malloc(8);
    int i;
    i = -p;
    i = ~p;
    i = +p;
    return i;
}
