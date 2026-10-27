void free(void *p);
void put(int ** _Nonnull pp, int ** _Nonnull src) { int *x = *src; free(x); *pp = x; }
