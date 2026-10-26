int main(void) {
    char c;
    void *v = &c;
    void *pv = &v;
    char **pc = pv;
    char *p = *pc;
    *p = 1;
    return c;
}
