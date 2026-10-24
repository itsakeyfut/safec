int *g(void);

int f(void) {
    int *p = g();
#define CHECKED /* the caller has checked p, so this went:
    if (!p) return 0; // */
    return *p;
}
