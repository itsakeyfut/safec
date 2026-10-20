#ifdef U junk
#else junk
int main(void) { return 0; }
#endif U
#ifdef U /* a comment is not a token */
#endif /* U */
