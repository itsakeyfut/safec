void f(char *const *pp) {
    const char **q = (const char **)pp;
    if (q) {
        *q = "ab";
    }
}
void set2(char **a, char **b);
void two_arguments_each_handed_read_only(void) {
    const char *x = "ab";
    const char *y = "cd";
    set2((char **)&x, (char **)&y);
}
