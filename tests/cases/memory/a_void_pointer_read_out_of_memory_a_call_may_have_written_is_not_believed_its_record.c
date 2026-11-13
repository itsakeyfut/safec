void stash(void **pv);
int main(void) {
    char c;
    void *v = &c;
    void **pv = &v;
    stash(pv);
    void *w = *pv;
    char *p = w;
    *p = 1;
    return c;
}
