void free(void *p);
void destroy(int ** _Nonnull pp) {
    free(*pp);
}
