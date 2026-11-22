void middle_of_three(const char ***ppp) {
    if (ppp && *ppp) {
        **ppp = "ab";
    }
}
void middle_of_a_row(const char *(**pp)[2]) {
    if (pp && *pp) {
        (**pp)[0] = "ab";
    }
}
void bottom_of_three(const char ***ppp) {
    if (ppp) {
        char ***q = (char ***)ppp;
        char **r = *q;
        if (r) {
            char *s = *r;
            if (s) {
                *s = 'x';
            }
        }
    }
}
