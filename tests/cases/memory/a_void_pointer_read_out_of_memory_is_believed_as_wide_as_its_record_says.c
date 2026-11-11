int main(void) {
    char c;
    void *v = &c;
    void **pv = &v;
    void *w = *pv;
    char *p = w;
    *p = 1;
    return c;
}
