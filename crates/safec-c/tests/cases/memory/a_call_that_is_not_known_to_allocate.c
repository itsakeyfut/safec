void *bar(void);
void free(void *p);
int f(void) {
    void *p = bar();
    free(p);
    free(p);
    return 0;
}
