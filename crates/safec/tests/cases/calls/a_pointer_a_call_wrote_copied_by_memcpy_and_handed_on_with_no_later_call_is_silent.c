void *memcpy(void *d, void *s, int n);
void get(int **out);
int use(int *p);
int f(void) {
    int *r = 0;
    get(&r);
    int *s = 0;
    memcpy(&s, &r, 8);
    if (s == 0) {
        return 0;
    }
    return use(s);
}
