// The copy spelling of `void *w = *pv;`: `w` is believed one byte wide, as
// its record says, so `*p = 1;` is proved and only the null doubt is left
// (#929).
void *memcpy(void *d, void *s, int n);
int main(void) {
    char c;
    void *v = &c;
    void **pv = &v;
    void *w;
    memcpy(&w, pv, 8);
    char *p = w;
    *p = 1;
    return c;
}
